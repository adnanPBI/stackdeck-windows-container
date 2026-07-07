//! Hyper-V backend for PyStack Runner.
//!
//! Replaces `hyperv_backend.py` — VM lifecycle, SSH orchestration,
//! container management, ISO generation, port proxy, SMB shares.

pub use pystack_types::{HyperVConfig, HyperVService};

use std::collections::HashMap;
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

fn create_command<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    cmd
}

const NONINTERACTIVE_SSH_OPTIONS: &[&str] = &[
    "StrictHostKeyChecking=accept-new",
    "BatchMode=yes",
    "NumberOfPasswordPrompts=0",
];

#[derive(Debug, thiserror::Error)]
pub enum HyperVError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("PowerShell error: {0}")]
    PowerShell(String),
    #[error("SSH error: {0}")]
    Ssh(String),
    #[error("config error: {0}")]
    Config(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Hyper-V backend manager.
pub struct HyperVManager {
    config: HyperVConfig,
}

impl HyperVManager {
    pub fn new(config: HyperVConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &HyperVConfig {
        &self.config
    }

    // -----------------------------------------------------------------------
    // Config management
    // -----------------------------------------------------------------------

    /// Load the Hyper-V config from disk.
    pub fn load_config() -> Result<HyperVConfig, HyperVError> {
        let path = pystack_types::hyperv_config_file();
        if !path.exists() {
            let mut cfg = HyperVConfig::default();
            cfg.smb_password = pystack_types::generate_random_password();
            Self::save_config(&cfg)?;
            return Ok(cfg);
        }
        let text = std::fs::read_to_string(&path)?;
        let mut cfg: HyperVConfig = serde_json::from_str(&text)?;
        if cfg.smb_password.is_empty() {
            cfg.smb_password = pystack_types::generate_random_password();
            Self::save_config(&cfg)?;
        }
        Ok(cfg)
    }

    /// Save the Hyper-V config to disk.
    pub fn save_config(config: &HyperVConfig) -> Result<(), HyperVError> {
        let path = pystack_types::hyperv_config_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(config)?;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // PowerShell wrapper
    // -----------------------------------------------------------------------

    /// Execute a PowerShell script and return stdout.
    pub fn ps(
        &self,
        script: &str,
        check: bool,
        timeout_secs: Option<u64>,
    ) -> Result<String, HyperVError> {
        let mut cmd = create_command("powershell");
        cmd.args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ]);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let (output, timed_out) = output_with_timeout(&mut cmd, timeout_secs)
            .map_err(|e| HyperVError::PowerShell(format!("failed to execute PowerShell: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if timed_out {
            return Err(HyperVError::PowerShell(timeout_error_message(
                "PowerShell",
                timeout_secs,
                &stdout,
                &stderr,
            )));
        }

        if check && !output.status.success() {
            let msg = stderr.trim().to_string();
            return Err(HyperVError::PowerShell(if msg.is_empty() {
                format!("PowerShell exited with code {:?}", output.status.code())
            } else {
                msg
            }));
        }

        Ok(stdout.trim().to_string())
    }

    // -----------------------------------------------------------------------
    // SSH wrapper
    // -----------------------------------------------------------------------

    /// Build the base SSH arguments for connecting to the VM.
    fn ssh_base(&self) -> Vec<String> {
        let cfg = &self.config;
        let mut args = vec!["ssh".to_string()];
        push_ssh_options(&mut args);
        args.extend(["-p".to_string(), cfg.ssh_port.to_string()]);
        if !cfg.ssh_identity.is_empty() {
            args.extend(["-i".to_string(), cfg.ssh_identity.clone()]);
        }
        args.push(format!("{}@{}", cfg.ssh_user, cfg.ssh_host));
        args
    }

    /// Execute a command over SSH and return stdout.
    pub fn ssh_stream(&self, command: &str, check: bool) -> Result<(), HyperVError> {
        let mut args = self.ssh_base();
        args.insert(1, "-t".to_string());
        args.push(command.to_string());
        self.run_cmd_stream(&args, check)
    }

    fn run_cmd_stream(&self, args: &[String], check: bool) -> Result<(), HyperVError> {
        let mut cmd = create_command(&args[0]);
        cmd.args(&args[1..])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let mut child = cmd
            .spawn()
            .map_err(|e| HyperVError::Ssh(format!("command failed: {}", e)))?;
        let status = child
            .wait()
            .map_err(|e| HyperVError::Ssh(format!("wait failed: {}", e)))?;

        if check && !status.success() {
            return Err(HyperVError::Ssh(format!("command failed: {}", args[0])));
        }

        Ok(())
    }

    pub fn ssh(&self, command: &str, check: bool) -> Result<String, HyperVError> {
        validate_config_for_shell(&self.config)?;
        let mut args = self.ssh_base();
        args.push(command.to_string());
        self.run_cmd(&args, check, None)
    }

    /// Execute a command over SSH with stdin input.
    pub fn ssh_input(
        &self,
        command: &str,
        input_text: &str,
        check: bool,
    ) -> Result<String, HyperVError> {
        let mut args = self.ssh_base();
        args.push(command.to_string());

        let mut child = create_command(&args[0])
            .args(&args[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| HyperVError::Ssh(format!("failed to spawn SSH: {}", e)))?;

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input_text.as_bytes());
        }

        let output = child
            .wait_with_output()
            .map_err(|e| HyperVError::Ssh(format!("SSH wait failed: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if check && !output.status.success() {
            return Err(HyperVError::Ssh(stderr.trim().to_string()));
        }

        Ok(stdout.trim().to_string())
    }

    /// Copy a file from localhost to the VM via SCP.
    pub fn scp_to_vm(&self, local: &Path, remote: &str) -> Result<(), HyperVError> {
        let cfg = &self.config;
        let mut args = vec![
            "scp".to_string(),
            "-P".to_string(),
            cfg.ssh_port.to_string(),
        ];
        push_ssh_options(&mut args);
        if local.is_dir() {
            args.push("-r".to_string());
        }
        if !cfg.ssh_identity.is_empty() {
            args.extend(["-i".to_string(), cfg.ssh_identity.clone()]);
        }
        args.push(local.to_string_lossy().to_string());
        args.push(format!("{}@{}:{}", cfg.ssh_user, cfg.ssh_host, remote));
        self.run_cmd(&args, true, None)?;
        Ok(())
    }

    /// Copy a file from the VM to localhost via SCP.
    pub fn scp_from_vm(&self, remote: &str, local: &Path) -> Result<(), HyperVError> {
        let cfg = &self.config;
        let mut args = vec![
            "scp".to_string(),
            "-P".to_string(),
            cfg.ssh_port.to_string(),
        ];
        push_ssh_options(&mut args);
        if !cfg.ssh_identity.is_empty() {
            args.extend(["-i".to_string(), cfg.ssh_identity.clone()]);
        }
        args.push(format!("{}@{}:{}", cfg.ssh_user, cfg.ssh_host, remote));
        args.push(local.to_string_lossy().to_string());
        self.run_cmd(&args, true, None)?;
        Ok(())
    }

    pub fn download_image(
        &self,
        url: &str,
        output: &str,
        sha256: Option<&str>,
        force: bool,
    ) -> Result<(), HyperVError> {
        if url.trim().is_empty() {
            return Err(HyperVError::Config("image URL must not be empty".into()));
        }
        let output_path = if output.trim().is_empty() {
            let file_name = url
                .rsplit('/')
                .next()
                .filter(|value| !value.is_empty())
                .unwrap_or("ubuntu-cloud.img");
            PathBuf::from(&self.config.vm_root)
                .join("images")
                .join(file_name)
        } else {
            PathBuf::from(output)
        };
        if output_path.exists() && !force {
            if let Some(expected) = sha256 {
                self.verify_file_sha256(&output_path, expected)?;
            }
            return Ok(());
        }
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let script = format!(
            r#"$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$url = '{url}'
$out = '{out}'
$tmp = "$out.partial"
if (Test-Path $tmp) {{ Remove-Item -Force $tmp }}
try {{
  Start-BitsTransfer -Source $url -Destination $tmp -ErrorAction Stop
}} catch {{
  Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing
}}
Move-Item -Force $tmp $out
"#,
            url = ps_quote(url),
            out = ps_quote(&output_path.to_string_lossy()),
        );
        self.ps(&script, true, None)?;
        if let Some(expected) = sha256 {
            self.verify_file_sha256(&output_path, expected)?;
        }
        Ok(())
    }

    pub fn create_vm(&self, iso: &str) -> Result<(), HyperVError> {
        let root = PathBuf::from(&self.config.vm_root);
        std::fs::create_dir_all(&root)?;
        let disk = root.join(format!("{}.vhdx", self.config.vm_name));
        let script = format!(
            r#"$ErrorActionPreference = 'Stop'
$name = '{name}'
$root = '{root}'
$disk = '{disk}'
$iso = '{iso}'
$switchName = '{switch}'
if (Get-VM -Name $name -ErrorAction SilentlyContinue) {{ throw "VM '$name' already exists" }}
if (!(Test-Path $root)) {{ New-Item -ItemType Directory -Force -Path $root | Out-Null }}
if (!(Test-Path $disk)) {{ New-VHD -Path $disk -Dynamic -SizeBytes {disk_gb}GB | Out-Null }}
New-VM -Name $name -Generation 2 -MemoryStartupBytes {memory_mb}MB -VHDPath $disk -SwitchName $switchName -Path $root | Out-Null
Set-VMProcessor -VMName $name -Count {cpus}
Set-VMMemory -VMName $name -DynamicMemoryEnabled $true -MinimumBytes 1024MB -StartupBytes {memory_mb}MB -MaximumBytes {memory_mb}MB
if ($iso -and (Test-Path $iso)) {{ Add-VMDvdDrive -VMName $name -Path $iso | Out-Null }}
try {{ Set-VMFirmware -VMName $name -EnableSecureBoot On -SecureBootTemplate 'MicrosoftUEFICertificateAuthority' }} catch {{}}
(Get-VM -Name $name).State
"#,
            name = ps_quote(&self.config.vm_name),
            root = ps_quote(&root.to_string_lossy()),
            disk = ps_quote(&disk.to_string_lossy()),
            iso = ps_quote(iso),
            switch = ps_quote(&self.config.switch_name),
            disk_gb = self.config.vm_disk_gb.max(10),
            memory_mb = self.config.vm_memory_mb.max(1024),
            cpus = self.config.vm_cpu_count.max(1),
        );
        self.ps(&script, true, Some(180))?;
        Ok(())
    }

    pub fn create_cloud_vm(
        &self,
        image_vhdx: &str,
        no_start: bool,
        discover_ip: bool,
        timeout: u32,
    ) -> Result<(), HyperVError> {
        let image = PathBuf::from(image_vhdx);
        if !image.exists() {
            return Err(HyperVError::Config(format!(
                "cloud image/VHDX does not exist: {}",
                image.display()
            )));
        }
        let public_key = self.ensure_ssh_key()?;
        let seed_iso = self.write_cloud_init_seed(&public_key)?;
        let root = PathBuf::from(&self.config.vm_root);
        std::fs::create_dir_all(&root)?;
        let disk = root.join(format!("{}.vhdx", self.config.vm_name));
        let script = format!(
            r#"$ErrorActionPreference = 'Stop'
$name = '{name}'
$root = '{root}'
$src = '{src}'
$disk = '{disk}'
$seed = '{seed}'
$switchName = '{switch}'
if (Get-VM -Name $name -ErrorAction SilentlyContinue) {{ throw "VM '$name' already exists. Stop/remove it or choose another --vm-name." }}
if (!(Test-Path $root)) {{ New-Item -ItemType Directory -Force -Path $root | Out-Null }}
Copy-Item -Force $src $disk
try {{ Resize-VHD -Path $disk -SizeBytes {disk_gb}GB }} catch {{ Write-Warning $_ }}
New-VM -Name $name -Generation 2 -MemoryStartupBytes {memory_mb}MB -VHDPath $disk -SwitchName $switchName -Path $root | Out-Null
Set-VMProcessor -VMName $name -Count {cpus}
Set-VMMemory -VMName $name -DynamicMemoryEnabled $true -MinimumBytes 1024MB -StartupBytes {memory_mb}MB -MaximumBytes {memory_mb}MB
Add-VMDvdDrive -VMName $name -Path $seed | Out-Null
try {{ Set-VMFirmware -VMName $name -EnableSecureBoot On -SecureBootTemplate 'MicrosoftUEFICertificateAuthority' }} catch {{}}
if (-not {no_start}) {{ Start-VM -Name $name | Out-Null }}
(Get-VM -Name $name).State
"#,
            name = ps_quote(&self.config.vm_name),
            root = ps_quote(&root.to_string_lossy()),
            src = ps_quote(&image.to_string_lossy()),
            disk = ps_quote(&disk.to_string_lossy()),
            seed = ps_quote(&seed_iso.to_string_lossy()),
            switch = ps_quote(&self.config.switch_name),
            disk_gb = self.config.vm_disk_gb.max(10),
            memory_mb = self.config.vm_memory_mb.max(1024),
            cpus = self.config.vm_cpu_count.max(1),
            no_start = if no_start { "$true" } else { "$false" },
        );
        self.ps(&script, true, Some(240))?;
        if discover_ip && !no_start {
            let ip = self.discover_ip(timeout)?;
            let mut cfg = self.config.clone();
            cfg.ssh_host = ip;
            HyperVManager::save_config(&cfg)?;
        }
        Ok(())
    }

    pub fn init_vm(
        &self,
        image_vhdx: &str,
        url: Option<&str>,
        sha256: Option<&str>,
        timeout: u32,
    ) -> Result<(), HyperVError> {
        let root = PathBuf::from(&self.config.vm_root);
        std::fs::create_dir_all(root.join("images"))?;
        let mut image_path = if image_vhdx.trim().is_empty() {
            let image_url = url.ok_or_else(|| {
                HyperVError::Config("--url is required when --image-vhdx is omitted".into())
            })?;
            let raw = root.join("images").join(
                image_url
                    .rsplit('/')
                    .next()
                    .filter(|v| !v.is_empty())
                    .unwrap_or("ubuntu-cloud.img"),
            );
            self.download_image(image_url, &raw.to_string_lossy(), sha256, false)?;
            raw
        } else {
            PathBuf::from(image_vhdx)
        };
        if !image_path.exists() {
            return Err(HyperVError::Config(format!(
                "image path does not exist: {}",
                image_path.display()
            )));
        }
        if image_path
            .extension()
            .and_then(|v| v.to_str())
            .map(|v| !v.eq_ignore_ascii_case("vhdx"))
            .unwrap_or(true)
        {
            image_path = self.convert_to_vhdx(&image_path)?;
        }
        self.create_cloud_vm(&image_path.to_string_lossy(), false, true, timeout)?;
        self.wait_for_ssh(timeout)?;
        self.bootstrap()?;
        Ok(())
    }

    pub fn ensure_ssh_key(&self) -> Result<String, HyperVError> {
        let mut identity = if self.config.ssh_identity.trim().is_empty() {
            pystack_types::registry_dir().join("id_ed25519")
        } else {
            PathBuf::from(&self.config.ssh_identity)
        };
        if identity.extension().and_then(|v| v.to_str()) == Some("pub") {
            identity.set_extension("");
        }
        if let Some(parent) = identity.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let public = identity.with_extension("pub");
        if !identity.exists() || !public.exists() {
            let args = vec![
                "ssh-keygen".to_string(),
                "-t".to_string(),
                "ed25519".to_string(),
                "-N".to_string(),
                "".to_string(),
                "-f".to_string(),
                identity.to_string_lossy().to_string(),
            ];
            self.run_cmd(&args, true, Some(60))?;
        }
        Ok(std::fs::read_to_string(public)?.trim().to_string())
    }

    pub fn bootstrap(&self) -> Result<(), HyperVError> {
        self.wait_for_ssh(300)?;
        let script = r#"set -eu
export DEBIAN_FRONTEND=noninteractive
sudo cloud-init status --wait || true
sudo apt-get update -y
sudo apt-get install -y ca-certificates curl gnupg containerd cifs-utils cloud-guest-utils uidmap iptables
sudo growpart /dev/sda 1 || true
sudo resize2fs /dev/sda1 || true
sudo systemctl enable --now containerd
if ! command -v nerdctl >/dev/null 2>&1; then
  ARCH=$(uname -m)
  case "$ARCH" in x86_64|amd64) NARCH=amd64 ;; aarch64|arm64) NARCH=arm64 ;; *) echo "unsupported arch $ARCH" >&2; exit 1 ;; esac
  VER=${NERDCTL_VERSION:-1.7.7}
  curl -fsSL "https://github.com/containerd/nerdctl/releases/download/v${VER}/nerdctl-${VER}-linux-${NARCH}.tar.gz" -o /tmp/nerdctl.tgz
  sudo tar Cxzvf /usr/local/bin /tmp/nerdctl.tgz nerdctl >/dev/null
fi
sudo mkdir -p /etc/containerd
if [ ! -s /etc/containerd/config.toml ]; then
  containerd config default | sudo tee /etc/containerd/config.toml >/dev/null
fi
sudo systemctl restart containerd
if ! sudo systemctl is-active --quiet containerd; then
  echo "containerd failed to start" >&2
  exit 1
fi
sudo nerdctl --namespace pystack version >/dev/null
"#;
        self.ssh_stream(script, true)?;
        self.apply_registry_mirrors()?;
        Ok(())
    }

    fn verify_file_sha256(&self, path: &Path, expected: &str) -> Result<(), HyperVError> {
        let expected = expected.trim().to_ascii_lowercase();
        if expected.is_empty() {
            return Ok(());
        }
        let script = format!(
            "(Get-FileHash -Algorithm SHA256 -Path '{}').Hash.ToLowerInvariant()",
            ps_quote(&path.to_string_lossy())
        );
        let actual = self
            .ps(&script, true, Some(60))?
            .trim()
            .to_ascii_lowercase();
        if actual != expected {
            return Err(HyperVError::Config(format!(
                "SHA256 mismatch for {}: expected {}, got {}",
                path.display(),
                expected,
                actual
            )));
        }
        Ok(())
    }

    fn convert_to_vhdx(&self, source: &Path) -> Result<PathBuf, HyperVError> {
        let output = PathBuf::from(&self.config.vm_root)
            .join("images")
            .join(format!(
                "{}.vhdx",
                source
                    .file_stem()
                    .and_then(|v| v.to_str())
                    .unwrap_or("cloud-image")
            ));
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let script = format!(
            r#"$ErrorActionPreference = 'Stop'
$src = '{src}'
$out = '{out}'
if (Test-Path $out) {{ Remove-Item -Force $out }}
$qemu = Get-Command qemu-img.exe -ErrorAction SilentlyContinue
if (-not $qemu) {{ $qemu = Get-Command qemu-img -ErrorAction SilentlyContinue }}
if (-not $qemu) {{ throw 'qemu-img is required to convert qcow2/img cloud images to VHDX. Install qemu-img or provide a pre-converted .vhdx with --image-vhdx.' }}
& $qemu.Source convert -O vhdx -o subformat=dynamic $src $out
Resize-VHD -Path $out -SizeBytes {disk_gb}GB
"#,
            src = ps_quote(&source.to_string_lossy()),
            out = ps_quote(&output.to_string_lossy()),
            disk_gb = self.config.vm_disk_gb.max(10),
        );
        self.ps(&script, true, Some(900))?;
        Ok(output)
    }

    fn write_cloud_init_seed(&self, public_key: &str) -> Result<PathBuf, HyperVError> {
        let seed_dir = PathBuf::from(&self.config.vm_root).join("seed");
        std::fs::create_dir_all(&seed_dir)?;
        let user_data = format!(
            r#"#cloud-config
users:
  - name: {user}
    groups: [adm, sudo]
    shell: /bin/bash
    sudo: ['ALL=(ALL) NOPASSWD:ALL']
    ssh_authorized_keys:
      - {key}
ssh_pwauth: false
package_update: true
packages:
  - ca-certificates
  - curl
  - gnupg
  - containerd
  - cifs-utils
  - cloud-guest-utils
  - uidmap
runcmd:
  - [ systemctl, enable, --now, containerd ]
  - [ growpart, /dev/sda, '1' ]
  - [ resize2fs, /dev/sda1 ]
"#,
            user = self.config.ssh_user,
            key = public_key.replace('\n', " "),
        );
        std::fs::write(seed_dir.join("user-data"), user_data)?;
        std::fs::write(
            seed_dir.join("meta-data"),
            format!(
                "instance-id: {}\nlocal-hostname: {}\n",
                self.config.vm_name, self.config.vm_name
            ),
        )?;
        let iso = PathBuf::from(&self.config.vm_root).join("seed.iso");
        let script = ISO_BUILD_SEED_PS
            .replace("@@SEED@@", &ps_quote(&seed_dir.to_string_lossy()))
            .replace("@@ISO@@", &ps_quote(&iso.to_string_lossy()));
        self.ps(&script, true, Some(180))?;
        Ok(iso)
    }

    pub fn discover_ip(&self, timeout: u32) -> Result<String, HyperVError> {
        let started = Instant::now();
        loop {
            let ip = self.vm_ip()?.trim().to_string();
            if !ip.is_empty() {
                validate_ssh_host(&ip)?;
                return Ok(ip);
            }
            if started.elapsed() > Duration::from_secs(timeout.max(1) as u64) {
                return Err(HyperVError::Config(format!(
                    "could not discover IP for VM '{}' within {}s",
                    self.config.vm_name, timeout
                )));
            }
            thread::sleep(Duration::from_secs(2));
        }
    }

    pub fn wait_for_ssh(&self, timeout: u32) -> Result<(), HyperVError> {
        let started = Instant::now();
        loop {
            if self.ssh_probe(3) {
                return Ok(());
            }
            if started.elapsed() > Duration::from_secs(timeout.max(1) as u64) {
                return Err(HyperVError::Ssh(format!(
                    "SSH did not become reachable at {}:{} within {}s",
                    self.config.ssh_host, self.config.ssh_port, timeout
                )));
            }
            thread::sleep(Duration::from_secs(2));
        }
    }

    pub fn repair(&self, timeout: u32) -> Result<(), HyperVError> {
        if self.vm_state().unwrap_or_default().trim() != "Running" {
            self.vm_start()?;
        }
        if self.config.ssh_host.trim().is_empty() {
            let ip = self.discover_ip(timeout)?;
            let mut cfg = self.config.clone();
            cfg.ssh_host = ip;
            HyperVManager::save_config(&cfg)?;
        }
        self.wait_for_ssh(timeout)?;
        self.bootstrap()?;
        self.apply_registry_mirrors()?;
        Ok(())
    }

    pub fn apply_registry_mirrors(&self) -> Result<(), HyperVError> {
        if self.config.registry_mirrors.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string(&self.config.registry_mirrors)?;
        let script = format!(
            r#"set -eu
sudo mkdir -p /etc/containerd/certs.d
cat > /tmp/pystack_mirrors.json <<'JSON'
{json}
JSON
python3 - <<'PY'
import json, os
with open('/tmp/pystack_mirrors.json', 'r', encoding='utf-8') as fh:
    mirrors = json.load(fh)
for registry, endpoints in mirrors.items():
    d = '/etc/containerd/certs.d/' + registry
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, 'hosts.toml'), 'w', encoding='utf-8') as f:
        for endpoint in endpoints:
            f.write('[host."%s"]\n  capabilities = ["pull", "resolve"]\n' % endpoint)
PY
sudo systemctl restart containerd
"#,
            json = json,
        );
        self.ssh(&script, true)?;
        Ok(())
    }

    pub fn mirror_list(&self) -> Result<String, HyperVError> {
        Ok(serde_json::to_string_pretty(&self.config.registry_mirrors)?)
    }

    pub fn mirror_set(&self, registry: &str, endpoints: &[String]) -> Result<(), HyperVError> {
        if registry.trim().is_empty() || endpoints.is_empty() {
            return Err(HyperVError::Config(
                "mirror registry and at least one endpoint are required".into(),
            ));
        }
        let mut cfg = self.config.clone();
        cfg.registry_mirrors
            .insert(registry.to_string(), endpoints.to_vec());
        HyperVManager::save_config(&cfg)?;
        HyperVManager::new(cfg).apply_registry_mirrors()?;
        Ok(())
    }

    pub fn mirror_remove(&self, registry: &str) -> Result<(), HyperVError> {
        let mut cfg = self.config.clone();
        cfg.registry_mirrors.remove(registry);
        HyperVManager::save_config(&cfg)?;
        Ok(())
    }

    pub fn image_prune(&self, all: bool) -> Result<String, HyperVError> {
        let mut args = vec!["image".to_string(), "prune".to_string(), "-f".to_string()];
        if all {
            args.push("-a".to_string());
        }
        self.ssh(&nerdctl_command(&self.config, &args), false)
    }

    pub fn image_login(
        &self,
        registry: &str,
        username: &str,
        password: &str,
    ) -> Result<String, HyperVError> {
        let cmd = format!(
            "printf %s {} | {}",
            shell_quote(password),
            nerdctl_command(
                &self.config,
                &["login", registry, "-u", username, "--password-stdin"]
            )
        );
        self.ssh(&cmd, true)
    }

    pub fn volume_prune(&self) -> Result<String, HyperVError> {
        self.ssh(
            &nerdctl_command(&self.config, &["volume", "prune", "-f"]),
            false,
        )
    }

    pub fn volume_inspect(&self, volumes: &[&str]) -> Result<String, HyperVError> {
        let mut args = vec!["volume".to_string(), "inspect".to_string()];
        args.extend(volumes.iter().map(|s| s.to_string()));
        self.ssh(&nerdctl_command(&self.config, &args), false)
    }

    pub fn network_inspect(&self, networks: &[&str]) -> Result<String, HyperVError> {
        let mut args = vec!["network".to_string(), "inspect".to_string()];
        args.extend(networks.iter().map(|s| s.to_string()));
        self.ssh(&nerdctl_command(&self.config, &args), false)
    }

    pub fn snapshot_create(&self, name: Option<&str>) -> Result<String, HyperVError> {
        let snap = name
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("snap-{}", unix_timestamp()));
        let script = format!(
            "Checkpoint-VM -Name '{}' -SnapshotName '{}'; '{}'",
            ps_quote(&self.config.vm_name),
            ps_quote(&snap),
            ps_quote(&snap)
        );
        self.ps(&script, true, Some(120))
    }

    pub fn snapshot_list(&self) -> Result<String, HyperVError> {
        let script = format!("Get-VMSnapshot -VMName '{}' | Select-Object Name,CreationTime | ConvertTo-Json -Depth 3", ps_quote(&self.config.vm_name));
        self.ps(&script, false, None)
    }

    pub fn snapshot_restore(&self, name: &str) -> Result<String, HyperVError> {
        let script = format!(
            "Restore-VMSnapshot -VMName '{}' -Name '{}' -Confirm:$false; (Get-VM -Name '{}').State",
            ps_quote(&self.config.vm_name),
            ps_quote(name),
            ps_quote(&self.config.vm_name)
        );
        self.ps(&script, true, Some(300))
    }

    pub fn snapshot_remove(&self, name: &str) -> Result<String, HyperVError> {
        let script = format!(
            "Remove-VMSnapshot -VMName '{}' -Name '{}' -Confirm:$false; 'removed'",
            ps_quote(&self.config.vm_name),
            ps_quote(name)
        );
        self.ps(&script, true, Some(120))
    }

    pub fn snapshot_export(&self, name: &str, output: &str) -> Result<String, HyperVError> {
        let script = format!(
            "Export-VMSnapshot -VMName '{}' -Name '{}' -Path '{}'; '{}'",
            ps_quote(&self.config.vm_name),
            ps_quote(name),
            ps_quote(output),
            ps_quote(output)
        );
        self.ps(&script, true, Some(900))
    }

    pub fn share_add(&self, path: &str, name: Option<&str>) -> Result<String, HyperVError> {
        let share_name = name
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("pystack_{}", unix_timestamp()));
        let user = if self.config.smb_user.is_empty() {
            "Everyone"
        } else {
            &self.config.smb_user
        };
        let script = format!(
            "if (!(Test-Path '{}')) {{ throw 'share path does not exist' }}; if (!(Get-SmbShare -Name '{}' -ErrorAction SilentlyContinue)) {{ New-SmbShare -Name '{}' -Path '{}' -ChangeAccess '{}' | Out-Null }}; '{}'",
            ps_quote(path), ps_quote(&share_name), ps_quote(&share_name), ps_quote(path), ps_quote(user), ps_quote(&share_name)
        );
        self.ps(&script, true, None)
    }

    pub fn share_mount(&self, name: &str) -> Result<String, HyperVError> {
        if self.config.windows_host.is_empty()
            || self.config.smb_user.is_empty()
            || self.config.smb_password.is_empty()
        {
            return Err(HyperVError::Config(
                "windows_host, smb_user, and smb_password are required for SMB mount".into(),
            ));
        }
        let share = project_slug(name);
        let remote = format!("/mnt/pystack-shares/{}", share);
        let cred = format!("/tmp/pystack-cifs-{}.cred", share);
        let unc = format!("//{}/{}", self.config.windows_host, name);
        let cmd = format!(
            "sudo mkdir -p {remote} && printf 'username=%s\npassword=%s\n' {user} {password} | sudo tee {cred} >/dev/null && sudo chmod 600 {cred} && if ! mountpoint -q {remote}; then sudo mount -t cifs {unc} {remote} -o credentials={cred},vers=3.0,iocharset=utf8,noserverino; fi; echo {remote}",
            remote = shell_quote(&remote),
            user = shell_quote(&self.config.smb_user),
            password = shell_quote(&self.config.smb_password),
            cred = shell_quote(&cred),
            unc = shell_quote(&unc),
        );
        self.ssh(&cmd, true)
    }

    /// Run a generic command and return stdout.
    fn run_cmd(
        &self,
        args: &[String],
        check: bool,
        timeout_secs: Option<u64>,
    ) -> Result<String, HyperVError> {
        let mut cmd = create_command(&args[0]);
        cmd.args(&args[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let (output, timed_out) = output_with_timeout(&mut cmd, timeout_secs)
            .map_err(|e| HyperVError::Ssh(format!("command failed: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if timed_out {
            return Err(HyperVError::Ssh(timeout_error_message(
                &args[0],
                timeout_secs,
                &stdout,
                &stderr,
            )));
        }

        if check && !output.status.success() {
            let msg = if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            };
            return Err(HyperVError::Ssh(msg.to_string()));
        }

        Ok(stdout.trim().to_string())
    }

    // -----------------------------------------------------------------------
    // VM lifecycle
    // -----------------------------------------------------------------------

    /// Start the VM.
    pub fn vm_start(&self) -> Result<String, HyperVError> {
        let name = &self.config.vm_name;
        let memory_mb = self.config.vm_memory_mb;
        let script = format!(
            r#"$ErrorActionPreference = 'Stop'
$name = '{}'
$configuredMb = {}
$os = Get-CimInstance Win32_OperatingSystem
$freeMb = [math]::Floor($os.FreePhysicalMemory / 1024)
$availableMb = $freeMb - 1024
$targetMb = if ($availableMb -lt $configuredMb) {{ $availableMb }} else {{ $configuredMb }}
if ($targetMb -lt 1024) {{ $targetMb = 1024 }}
Set-VMMemory -VMName $name -DynamicMemoryEnabled $true -MinimumBytes 1024MB -StartupBytes ($targetMb * 1MB) -MaximumBytes ($configuredMb * 1MB) -ErrorAction SilentlyContinue
Start-VM -Name $name
(Get-VM -Name $name).State"#,
            name.replace('\'', "''"),
            memory_mb
        );
        self.ps(&script, true, Some(60))
    }

    /// Stop the VM.
    pub fn vm_stop(&self) -> Result<String, HyperVError> {
        let name = &self.config.vm_name;
        let script = format!(
            "Stop-VM -Name '{}' -Force; (Get-VM -Name '{}').State",
            name.replace('\'', "''"),
            name.replace('\'', "''")
        );
        self.ps(&script, true, Some(60))
    }

    /// Get the VM state.
    pub fn vm_state(&self) -> Result<String, HyperVError> {
        let name = &self.config.vm_name;
        let script = format!(
            "(Get-VM -Name '{}' -ErrorAction SilentlyContinue).State",
            name.replace('\'', "''")
        );
        self.ps(&script, false, None)
    }

    /// Get the VM IP address.
    pub fn vm_ip(&self) -> Result<String, HyperVError> {
        let name = &self.config.vm_name;
        let script = format!(
            r#"
try {{
  $vm = Get-VM -Name '{}' -ErrorAction Stop
}} catch {{
  exit 0
}}
$ip = ($vm.NetworkAdapters | Select-Object -ExpandProperty IPAddresses | Where-Object {{ $_ -match '^\d+\.\d+\.\d+\.\d+$' }} | Select-Object -First 1)
if ($ip) {{
  $ip
  exit 0
}}
$mac = ($vm.NetworkAdapters | Select-Object -First 1 -ExpandProperty MacAddress)
if ($mac) {{
  $normalized = (($mac -replace '[^0-9A-Fa-f]', '').ToUpper() -replace '(..)(?=.)', '$1-')
  Get-NetNeighbor -AddressFamily IPv4 -ErrorAction SilentlyContinue |
    Where-Object {{ $_.LinkLayerAddress -and $_.LinkLayerAddress.ToUpper() -eq $normalized -and $_.IPAddress -match '^\d+\.\d+\.\d+\.\d+$' }} |
    Select-Object -First 1 -ExpandProperty IPAddress
}}
"#,
            name.replace('\'', "''")
        );
        self.ps(&script, false, None)
    }

    /// Check if SSH is reachable.
    pub fn ssh_probe(&self, timeout_secs: u64) -> bool {
        if self.config.ssh_host.is_empty() {
            return false;
        }
        let mut args = self.ssh_base();
        args.insert(1, "-o".to_string());
        args.insert(2, format!("ConnectTimeout={}", timeout_secs.max(1)));
        args.push("true".to_string());
        self.run_cmd(&args, false, Some(timeout_secs)).is_ok()
    }

    // -----------------------------------------------------------------------
    // Container operations
    // -----------------------------------------------------------------------

    /// Start a service container on the Hyper-V VM.

    /// Build a container image natively on the Hyper-V VM
    pub fn build_image(
        &self,
        svc: &pystack_types::HyperVService,
        tag: &str,
    ) -> Result<(), HyperVError> {
        let Some(build_val) = &svc.build else {
            return Ok(());
        };

        let mut context = svc.root.clone();
        let mut dockerfile = None;
        let mut target = None;
        let mut args_map = std::collections::HashMap::new();

        if let Some(b_obj) = build_val.as_object() {
            if let Some(c) = b_obj.get("context").and_then(|v| v.as_str()) {
                context = context.join(c);
            }
            if let Some(f) = b_obj
                .get("dockerfile")
                .or_else(|| b_obj.get("file"))
                .and_then(|v| v.as_str())
            {
                dockerfile = Some(f.to_string());
            }
            if let Some(t) = b_obj.get("target").and_then(|v| v.as_str()) {
                target = Some(t.to_string());
            }
            if let Some(a) = b_obj.get("args").and_then(|v| v.as_object()) {
                for (k, v) in a {
                    args_map.insert(k.clone(), v.as_str().unwrap_or("").to_string());
                }
            }
        } else if let Some(c) = build_val.as_str() {
            context = context.join(c);
        }

        context = std::fs::canonicalize(&context).unwrap_or(context);
        if !context.exists() {
            return Err(HyperVError::Config(format!(
                "Build context not found: {}",
                context.display()
            )));
        }

        let mut excludes = vec![
            ".pystack".to_string(),
            ".pystack-*".to_string(),
            "__pycache__".to_string(),
            ".git".to_string(),
        ];
        let d_ignore = context.join(".dockerignore");
        if let Ok(file_content) = std::fs::read_to_string(&d_ignore) {
            for line in file_content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                    continue;
                }
                let pattern = line.trim_start_matches('/');
                if !pattern.is_empty() {
                    excludes.push(pattern.to_string());
                    if !pattern.contains('/') && !pattern.chars().any(|c| "*?[".contains(c)) {
                        excludes.push(format!("*/{}", pattern));
                    }
                }
            }
        }

        let remote_dir = format!(
            "/tmp/pystack-build-{}",
            container_name(&svc.project, &svc.name)
        );
        let temp_dir = std::env::temp_dir().join(format!(
            "pystack-build-{}",
            container_name(&svc.project, &svc.name)
        ));
        std::fs::create_dir_all(&temp_dir)
            .map_err(|e| HyperVError::Ssh(format!("Failed to create temp dir: {}", e)))?;

        struct TempFileGuard(std::path::PathBuf);
        impl Drop for TempFileGuard {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let archive_path = temp_dir.join("context.tar.gz");
        let _cleanup_guard = TempFileGuard(archive_path.clone());

        let mut tar_cmd = create_command("tar");
        for ex in excludes {
            tar_cmd.arg(format!("--exclude={}", ex));
        }
        tar_cmd.args(&[
            "-czf",
            archive_path.to_str().unwrap(),
            "-C",
            context.to_str().unwrap(),
            ".",
        ]);
        let tar_status = tar_cmd
            .status()
            .map_err(|e| HyperVError::Ssh(format!("Failed to run tar: {}", e)))?;
        if !tar_status.success() {
            return Err(HyperVError::Ssh("Tar command failed".into()));
        }

        self.ssh(
            &format!(
                "rm -rf {} && mkdir -p {}",
                shell_quote(&remote_dir),
                shell_quote(&remote_dir)
            ),
            true,
        )?;
        self.scp_to_vm(&archive_path, &format!("{}/context.tar.gz", remote_dir))?;
        self.ssh(
            &format!("cd {} && tar -xzf context.tar.gz", shell_quote(&remote_dir)),
            true,
        )?;

        let mut nerdctl_args = vec!["build".to_string()];
        if let Some(f) = dockerfile {
            nerdctl_args.push("-f".to_string());
            nerdctl_args.push(f);
        }
        if let Some(t) = target {
            nerdctl_args.push("--target".to_string());
            nerdctl_args.push(t);
        }
        for (k, v) in args_map {
            nerdctl_args.push("--build-arg".to_string());
            nerdctl_args.push(format!("{}={}", k, v));
        }
        nerdctl_args.push("-t".to_string());
        nerdctl_args.push(tag.to_string());
        nerdctl_args.push(".".to_string());

        let cmd = nerdctl_command(&self.config, &nerdctl_args);
        println!("Building image {} for {}...", tag, svc.name);
        self.ssh_stream(&format!("cd {} && {}", shell_quote(&remote_dir), cmd), true)?;

        Ok(())
    }

    pub fn start_service(
        &self,
        svc: &pystack_types::HyperVService,
        build: bool,
    ) -> Result<String, HyperVError> {
        validate_config_for_shell(&self.config)?;
        let cfg = &self.config;
        if cfg.ssh_host.is_empty() {
            return Err(HyperVError::Config(
                "Hyper-V backend is not configured with ssh_host".into(),
            ));
        }

        if !self.ssh_probe(1) {
            println!("VM is offline or unreachable. Starting VM...");
            self.vm_start()?;

            let mut attempts = 0;
            while !self.ssh_probe(1) {
                if attempts >= 120 {
                    return Err(HyperVError::Ssh(
                        "Timed out waiting for VM SSH to become available".into(),
                    ));
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
                attempts += 1;
            }
        }

        let cname = container_name(&svc.project, &svc.name);
        let image = if svc.image.is_empty() {
            format!("pystack/{}-{}:latest", svc.project, svc.name)
        } else {
            svc.image.clone()
        };

        // Build image if specified, otherwise pull
        if build && svc.build.is_some() {
            self.build_image(svc, &image)?;
        } else if !image.is_empty() {
            let check = self
                .ssh(&nerdctl_command(cfg, &["image", "inspect", &image]), false)
                .unwrap_or_default();
            if check.is_empty() {
                if svc.build.is_some() {
                    // Try to build if missing
                    self.build_image(svc, &image)?;
                } else {
                    println!("Pulling image {}...", image);
                    self.ssh_stream(&nerdctl_command(cfg, &["pull", &image]), true)?;
                }
            }
        }

        // Remove existing container
        self.ssh(&nerdctl_command(cfg, &["rm", "-f", &cname]), false)
            .ok();

        let network = project_network_name(&svc.project);
        self.ensure_project_network(&network)?;

        // Build nerdctl run args
        let mut args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            cname.clone(),
            "--hostname".to_string(),
            service_hostname(&svc.name),
            "--network".to_string(),
            network,
        ];

        let restart = if svc.restart == "always" || svc.restart == "unless-stopped" {
            "always"
        } else {
            "no"
        };
        args.extend(["--restart".to_string(), restart.to_string()]);

        for (key, value) in &svc.env {
            args.extend(["-e".to_string(), format!("{}={}", key, value)]);
        }

        if let Some(hc) = &svc.healthcheck {
            if hc.r#type != "none" {
                if let Some(test) = &hc.test {
                    if let Some(cmd) = compose_health_command(test) {
                        args.extend(["--health-cmd".to_string(), cmd]);
                        args.extend([
                            "--health-interval".to_string(),
                            format!("{}s", hc.interval_seconds.max(1)),
                        ]);
                        args.extend([
                            "--health-timeout".to_string(),
                            format!("{}s", hc.timeout_seconds.max(1)),
                        ]);
                        args.extend([
                            "--health-retries".to_string(),
                            hc.retries.max(1).to_string(),
                        ]);
                        if hc.start_period_seconds > 0 {
                            args.extend([
                                "--health-start-period".to_string(),
                                format!("{}s", hc.start_period_seconds),
                            ]);
                        }
                    }
                }
            }
        }

        for port in &svc.ports {
            if is_udp_port(port) {
                return Err(HyperVError::Config(format!(
                    "UDP port publishing is not supported by the production MVP TCP portproxy backend: {}",
                    port
                )));
            }
            if let Some((_, host_port, container_port)) = parse_port(port) {
                args.extend([
                    "-p".to_string(),
                    format!("{}:{}", host_port, container_port),
                ]);
            }
        }

        for volume in &svc.volumes {
            let prepared = self.ensure_volume_mounts(svc, volume)?;
            args.extend(["-v".to_string(), prepared]);
        }

        args.push(image);

        if let Some(cmd) = &svc.command {
            match cmd {
                serde_json::Value::Array(arr) => {
                    for part in arr {
                        if let Some(s) = part.as_str() {
                            args.push(s.to_string());
                        }
                    }
                }
                serde_json::Value::String(s) => {
                    args.extend(["sh".to_string(), "-lc".to_string(), s.clone()]);
                }
                _ => {}
            }
        }

        let cmd_str = nerdctl_command(cfg, &args);
        let result = self.ssh(&cmd_str, true)?;

        // Inject healthcheck waiting loop
        if let Some(hc) = &svc.healthcheck {
            if hc.test.is_some() && hc.r#type != "none" {
                println!("{}: waiting for healthcheck...", svc.name);
                let cname = container_name(&svc.project, &svc.name);
                let max_duration = std::time::Duration::from_secs(
                    (hc.start_period_seconds
                        + (hc.interval_seconds + hc.timeout_seconds) * hc.retries
                        + 10) as u64,
                );
                let start_time = std::time::Instant::now();
                let mut healthy = false;

                while start_time.elapsed() < max_duration {
                    let check_args = vec!["inspect".to_string(), cname.clone()];
                    let check_cmd = nerdctl_command(cfg, &check_args);
                    let out = self
                        .ssh(&format!("{} 2>/dev/null || true", check_cmd), false)
                        .unwrap_or_default();
                    if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(&out) {
                        if let Some(item) = parsed.first() {
                            if let Some(state) = item.get("State") {
                                if let Some(health) = state.get("Health") {
                                    if let Some(status) =
                                        health.get("Status").and_then(|v| v.as_str())
                                    {
                                        if status == "healthy" {
                                            healthy = true;
                                            break;
                                        } else if status == "unhealthy" {
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
                if !healthy {
                    return Err(HyperVError::Ssh(format!(
                        "{}: healthcheck failed or timed out",
                        svc.name
                    )));
                }
            }
        }

        // Set up port proxy
        self.ensure_portproxy(svc)?;

        Ok(result)
    }

    fn ensure_project_network(&self, name: &str) -> Result<(), HyperVError> {
        if name.is_empty() {
            return Err(HyperVError::Config(
                "cannot create Hyper-V container network for empty project name".into(),
            ));
        }

        let inspect = self
            .ssh(
                &nerdctl_command(&self.config, &["network", "inspect", name]),
                false,
            )
            .unwrap_or_default();
        if !inspect.trim().is_empty() {
            return Ok(());
        }

        self.network_create(name)?;
        Ok(())
    }

    fn ensure_volume_mounts(
        &self,
        svc: &HyperVService,
        volume: &str,
    ) -> Result<String, HyperVError> {
        let Some(spec) = parse_volume_spec(volume) else {
            return Ok(volume.to_string());
        };
        if !is_bind_source(&spec.source) {
            return Ok(volume.to_string());
        }

        let local = local_bind_source(&svc.root, &spec.source);
        if !local.exists() {
            return Err(HyperVError::Config(format!(
                "bind mount source does not exist: {}",
                local.display()
            )));
        }

        if !self.config.smb_user.is_empty()
            && !self.config.smb_password.is_empty()
            && !self.config.windows_host.is_empty()
        {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            local.to_string_lossy().hash(&mut hasher);
            let share_name = format!("pystack_{:x}", hasher.finish());
            let windows_path = local.to_string_lossy();
            let smb_user = &self.config.smb_user;
            let smb_password = &self.config.smb_password;
            let windows_host = &self.config.windows_host;

            let ps_script = format!(
                "$name = '{}'; $path = '{}'; if (-not (Get-NetFirewallRule -DisplayGroup 'File and Printer Sharing' -Enabled True -ErrorAction SilentlyContinue)) {{ Enable-NetFirewallRule -DisplayGroup 'File and Printer Sharing' -ErrorAction SilentlyContinue | Out-Null }}; if (-not (Get-SmbShare -Name $name -ErrorAction SilentlyContinue)) {{ New-SmbShare -Name $name -Path $path -ChangeAccess '{}' | Out-Null }}",
                ps_quote(&share_name), ps_quote(&windows_path), ps_quote(smb_user)
            );
            self.ps(&ps_script, true, None)?;

            let remote = format!("/mnt/pystack-shares/{}", share_name);
            let cred = format!("/etc/pystack/cifs-{}.cred", share_name);
            let unc = format!("//{}/{}", windows_host, share_name);
            let mount_cmd = format!(
                "sudo mkdir -p {remote} && sudo mkdir -p /etc/pystack && printf 'username=%s\npassword=%s\n' {user} {password} | sudo tee {cred} >/dev/null && sudo chmod 600 {cred} && if ! mountpoint -q {remote}; then sudo mount -t cifs {unc} {remote} -o credentials={cred},vers=3.0,iocharset=utf8,noserverino || exit 1; fi",
                remote=remote, user=shell_quote(smb_user), password=shell_quote(smb_password), cred=cred, unc=unc
            );
            self.ssh(&mount_cmd, true)?;

            let mut next = format!("{}:{}", remote, spec.target);
            if let Some(mode) = spec.mode {
                next.push(':');
                next.push_str(&mode);
            }
            return Ok(next);
        }

        Err(HyperVError::Config(format!(
            "live bind mount requires SMB config. Configure --windows-host, --smb-user, and --smb-password before using bind source {}",
            local.display()
        )))
    }

    /// Stop a service container.
    pub fn stop_service(&self, svc: &HyperVService) -> Result<String, HyperVError> {
        let name = &svc.name;
        if !self.ssh_probe(1) {
            println!("{}: VM is offline, skipping removal", name);
            return Ok(format!("{}: VM is offline, skipping removal", name));
        }

        let cname = container_name(&svc.project, &svc.name);
        let _ = self.remove_portproxy(svc);
        let res = self.ssh(&nerdctl_command(&self.config, &["rm", "-f", &cname]), false);

        // Also ensure any leftover processes binding to the ports are killed
        for port in &svc.ports {
            if let Some((_, host_port, _)) = parse_port(port) {
                let script = format!("sudo fuser -k -9 {}/tcp || true", host_port);
                let _ = self.ssh(&script, false);
            }
        }
        res
    }

    /// List containers.
    pub fn container_ps(&self) -> Result<String, HyperVError> {
        self.ssh(&nerdctl_command(&self.config, &["ps", "-a"]), false)
    }

    /// Execute a command inside a container.
    pub fn exec_container(&self, container: &str, command: &[&str]) -> Result<String, HyperVError> {
        let mut args = vec!["exec".to_string(), container.to_string()];
        args.extend(command.iter().map(|s| s.to_string()));
        self.ssh(&nerdctl_command(&self.config, &args), false)
    }

    /// Get container logs.
    pub fn logs(&self, container: &str, tail: u32) -> Result<String, HyperVError> {
        let args = vec![
            "logs".to_string(),
            "--tail".to_string(),
            tail.to_string(),
            container.to_string(),
        ];
        self.ssh(&nerdctl_command(&self.config, &args), false)
    }

    /// Inspect containers.
    pub fn inspect_containers(
        &self,
        names: &[&str],
    ) -> Result<HashMap<String, serde_json::Value>, HyperVError> {
        let mut result = HashMap::new();
        for name in names {
            let raw = self
                .ssh(
                    &format!(
                        "{} 2>/dev/null || true",
                        nerdctl_command(&self.config, &["inspect", name])
                    ),
                    false,
                )
                .unwrap_or_default();

            if raw.is_empty() {
                result.insert(
                    name.to_string(),
                    serde_json::json!({"exists": false, "status": "stopped"}),
                );
                continue;
            }

            match serde_json::from_str::<Vec<serde_json::Value>>(&raw) {
                Ok(parsed) => {
                    let item = parsed.first().cloned().unwrap_or(serde_json::json!({}));
                    let state = item.get("State").cloned().unwrap_or(serde_json::json!({}));
                    let running = state
                        .get("Running")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let mut status = state
                        .get("Status")
                        .and_then(|v| v.as_str())
                        .unwrap_or(if running { "running" } else { "stopped" })
                        .to_string();

                    if let Some(health_status) =
                        state.pointer("/Health/Status").and_then(|v| v.as_str())
                    {
                        if health_status == "unhealthy" {
                            status = "unhealthy".to_string();
                        }
                    }

                    let exit_code = state.get("ExitCode").and_then(|v| v.as_i64()).unwrap_or(0);

                    result.insert(
                        name.to_string(),
                        serde_json::json!({
                            "exists": true,
                            "status": status,
                            "running": running,
                            "exit_code": exit_code,
                        }),
                    );
                }
                Err(_) => {
                    result.insert(
                        name.to_string(),
                        serde_json::json!({"exists": true, "status": "unknown"}),
                    );
                }
            }
        }
        Ok(result)
    }

    pub fn container_health_status(&self, name: &str) -> Result<String, HyperVError> {
        let raw = self.ssh(
            &format!(
                "{} 2>/dev/null || true",
                nerdctl_command(&self.config, &["inspect", name])
            ),
            false,
        )?;
        if raw.trim().is_empty() {
            return Ok("missing".to_string());
        }
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap_or_default();
        let Some(item) = parsed.first() else {
            return Ok("unknown".to_string());
        };
        let state = item.get("State").cloned().unwrap_or_default();
        if let Some(status) = state.pointer("/Health/Status").and_then(|v| v.as_str()) {
            return Ok(status.to_string());
        }
        let running = state
            .get("Running")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(if running { "running" } else { "stopped" }.to_string())
    }

    // -----------------------------------------------------------------------
    // Image operations
    // -----------------------------------------------------------------------

    /// List images.
    pub fn image_list(&self, all: bool) -> Result<String, HyperVError> {
        let mut args = vec!["images".to_string()];
        if all {
            args.push("--all".to_string());
        }
        self.ssh(&nerdctl_command(&self.config, &args), false)
    }

    /// Remove images.
    pub fn image_remove(&self, images: &[&str], force: bool) -> Result<String, HyperVError> {
        let mut args = vec!["rmi".to_string()];
        if force {
            args.push("-f".to_string());
        }
        args.extend(images.iter().map(|s| s.to_string()));
        self.ssh(&nerdctl_command(&self.config, &args), false)
    }

    // -----------------------------------------------------------------------
    // Volume operations
    // -----------------------------------------------------------------------

    /// Create a volume.
    pub fn volume_create(&self, name: &str) -> Result<String, HyperVError> {
        self.ssh(
            &nerdctl_command(&self.config, &["volume", "create", name]),
            true,
        )
    }

    /// List volumes.
    pub fn volume_list(&self) -> Result<String, HyperVError> {
        self.ssh(&nerdctl_command(&self.config, &["volume", "ls"]), false)
    }

    /// Remove volumes.
    pub fn volume_remove(&self, volumes: &[&str], force: bool) -> Result<String, HyperVError> {
        let mut args = vec!["volume".to_string(), "rm".to_string()];
        if force {
            args.push("-f".to_string());
        }
        args.extend(volumes.iter().map(|s| s.to_string()));
        self.ssh(&nerdctl_command(&self.config, &args), false)
    }

    // -----------------------------------------------------------------------
    // Network operations
    // -----------------------------------------------------------------------

    /// Create a network.
    pub fn network_create(&self, name: &str) -> Result<String, HyperVError> {
        self.ssh(
            &nerdctl_command(&self.config, &["network", "create", name]),
            true,
        )
    }

    /// List networks.
    pub fn network_list(&self) -> Result<String, HyperVError> {
        self.ssh(&nerdctl_command(&self.config, &["network", "ls"]), false)
    }

    /// Remove networks.
    pub fn network_remove(&self, networks: &[&str]) -> Result<String, HyperVError> {
        let args: Vec<String> = ["network", "rm"]
            .iter()
            .chain(networks.iter())
            .map(|s| s.to_string())
            .collect();
        self.ssh(&nerdctl_command(&self.config, &args), false)
    }

    // -----------------------------------------------------------------------
    // Port proxy
    // -----------------------------------------------------------------------

    /// Set up Windows port proxy for service ports.
    pub fn ensure_portproxy(&self, svc: &pystack_types::HyperVService) -> Result<(), HyperVError> {
        if !self.config.portproxy {
            return Ok(());
        }
        for port in &svc.ports {
            if is_udp_port(port) {
                return Err(HyperVError::Config(format!(
                    "UDP port publishing is not supported by the production MVP TCP portproxy backend: {}",
                    port
                )));
            }
            if let Some((host_ip, host_port, _)) = parse_port(port) {
                validate_ssh_host(&self.config.ssh_host)?;
                let mut script = format!(
                    "netsh interface portproxy delete v4tov4 listenaddress={} listenport={}; \
                      netsh interface portproxy add v4tov4 listenaddress={} listenport={} \
                      connectaddress={} connectport={}",
                    host_ip, host_port, host_ip, host_port, self.config.ssh_host, host_port
                );
                if host_ip == "0.0.0.0" {
                    script = format!(
                        "if (-not (Get-NetFirewallRule -DisplayName 'PyStack Port {}' -ErrorAction SilentlyContinue)) {{ New-NetFirewallRule -DisplayName 'PyStack Port {}' -Direction Inbound -LocalPort {} -Protocol TCP -Action Allow | Out-Null }}; {}",
                        host_port, host_port, host_port, script
                    );
                }
                self.ps(&script, true, None)?;
            }
        }
        Ok(())
    }

    /// Remove Windows port proxy for service ports.
    pub fn remove_portproxy(&self, svc: &pystack_types::HyperVService) -> Result<(), HyperVError> {
        for port in &svc.ports {
            if let Some((host_ip, host_port, _)) = parse_port(port) {
                let mut script = format!(
                    "netsh interface portproxy delete v4tov4 listenaddress={} listenport={}",
                    host_ip, host_port
                );
                if host_ip == "0.0.0.0" {
                    script = format!(
                        "Remove-NetFirewallRule -DisplayName 'PyStack Port {}' -ErrorAction SilentlyContinue | Out-Null; {}",
                        host_port, script
                    );
                }
                self.ps(&script, false, None).ok();
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Health check
    // -----------------------------------------------------------------------

    /// Run a runtime health check.
    pub fn runtime_health_check(&self) -> Result<HashMap<String, serde_json::Value>, HyperVError> {
        let mut health = HashMap::new();
        health.insert("vm_name".into(), serde_json::json!(self.config.vm_name));
        health.insert(
            "vm_state".into(),
            serde_json::json!(self.vm_state().unwrap_or_default()),
        );
        health.insert("ssh_host".into(), serde_json::json!(self.config.ssh_host));
        health.insert(
            "ssh".into(),
            serde_json::json!(if self.ssh_probe(5) {
                "reachable"
            } else {
                "unreachable"
            }),
        );

        if self.ssh_probe(5) {
            let containerd = self
                .ssh("systemctl is-active containerd 2>/dev/null || true", false)
                .unwrap_or_default();
            health.insert("containerd".into(), serde_json::json!(containerd));

            let nerdctl_ver = self
                .ssh("nerdctl --version 2>/dev/null || true", false)
                .unwrap_or_default();
            health.insert("nerdctl".into(), serde_json::json!(nerdctl_ver));
        }

        Ok(health)
    }

    // -----------------------------------------------------------------------
    // Preflight / Doctor
    // -----------------------------------------------------------------------

    /// Run Hyper-V preflight checks.
    pub fn preflight(&self) -> HashMap<String, String> {
        let mut result = HashMap::new();

        result.insert("platform".into(), "windows".into());

        let admin = self.ps(
            "([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)",
            false, None,
        ).unwrap_or_default();
        result.insert("admin".into(), admin.to_lowercase());

        let hyperv_cmdlets = self.ps(
            "Get-Command New-VM -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name",
            false, None,
        ).unwrap_or_default();
        result.insert(
            "hyperv_cmdlets".into(),
            if hyperv_cmdlets.is_empty() {
                "missing".into()
            } else {
                "available".into()
            },
        );

        let ssh_check = self.run_cmd(&["where.exe".to_string(), "ssh".to_string()], false, None);
        result.insert(
            "ssh".into(),
            if ssh_check.is_ok() {
                "available".into()
            } else {
                "missing".into()
            },
        );

        let scp_check = self.run_cmd(&["where.exe".to_string(), "scp".to_string()], false, None);
        result.insert(
            "scp".into(),
            if scp_check.is_ok() {
                "available".into()
            } else {
                "missing".into()
            },
        );

        result.insert("vm_name".into(), self.config.vm_name.clone());
        result.insert(
            "ssh_target".into(),
            format!(
                "{}@{}:{}",
                self.config.ssh_user, self.config.ssh_host, self.config.ssh_port
            ),
        );

        result
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

fn push_ssh_options(args: &mut Vec<String>) {
    for option in NONINTERACTIVE_SSH_OPTIONS {
        args.extend(["-o".to_string(), option.to_string()]);
    }
}

fn output_with_timeout(
    command: &mut Command,
    timeout_secs: Option<u64>,
) -> std::io::Result<(Output, bool)> {
    let Some(timeout_secs) = timeout_secs else {
        return command.output().map(|output| (output, false));
    };

    let timeout = Duration::from_secs(timeout_secs.max(1));
    let started = Instant::now();
    let mut child = command.spawn()?;

    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output().map(|output| (output, false));
        }

        if started.elapsed() >= timeout {
            let _ = child.kill();
            return child.wait_with_output().map(|output| (output, true));
        }

        let remaining = timeout
            .checked_sub(started.elapsed())
            .unwrap_or_else(|| Duration::from_millis(0));
        thread::sleep(remaining.min(Duration::from_millis(50)));
    }
}

fn timeout_error_message(
    command_name: &str,
    timeout_secs: Option<u64>,
    stdout: &str,
    stderr: &str,
) -> String {
    let timeout_secs = timeout_secs.unwrap_or_default().max(1);
    let mut msg = format!("{command_name} timed out after {timeout_secs} seconds");
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    if !detail.is_empty() {
        msg.push_str(": ");
        msg.push_str(detail);
    }
    msg
}

/// Generate a container name from project and service.
pub fn container_name(project: &str, service: &str) -> String {
    let raw = format!("pystack-{}-{}", project, service).to_lowercase();
    let re = regex::Regex::new(r"[^a-z0-9_.-]+").unwrap();
    let cleaned = re.replace_all(&raw, "-").to_string();
    let trimmed = cleaned.trim_matches('-');
    trimmed.chars().take(120).collect()
}

/// Generate a project-scoped network name.
pub fn project_network_name(project: &str) -> String {
    let slug = project_slug(project);
    if slug.is_empty() {
        String::new()
    } else {
        format!("pystack-{}", slug)
    }
}

fn service_hostname(service: &str) -> String {
    let slug = project_slug(service);
    if slug.is_empty() {
        "service".to_string()
    } else {
        slug
    }
}

fn compose_health_command(test: &serde_json::Value) -> Option<String> {
    match test {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        serde_json::Value::Array(items) => {
            let mut parts: Vec<String> = items
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            if parts.is_empty() {
                return None;
            }
            let first = parts.remove(0);
            match first.as_str() {
                "NONE" => None,
                "CMD" => Some(
                    parts
                        .into_iter()
                        .map(|p| shell_quote(&p))
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                "CMD-SHELL" => Some(parts.join(" ")),
                _ => {
                    parts.insert(0, first);
                    Some(
                        parts
                            .into_iter()
                            .map(|p| shell_quote(&p))
                            .collect::<Vec<_>>()
                            .join(" "),
                    )
                }
            }
        }
        _ => None,
    }
}

fn is_udp_port(port: &str) -> bool {
    port.trim().to_ascii_lowercase().ends_with("/udp")
}

/// Parse a TCP port string like "8080:80" into (host_ip, host_port, container_port).
pub fn parse_port(port: &str) -> Option<(String, u16, u16)> {
    let text = port.trim().trim_matches('"').trim_matches('\'');
    if text.is_empty() {
        return None;
    }
    let text = if let Some(idx) = text.find('/') {
        &text[..idx]
    } else {
        text
    };
    let parts: Vec<&str> = text.split(':').collect();
    match parts.len() {
        1 => {
            let port: u16 = parts[0].parse().ok()?;
            Some(("127.0.0.1".to_string(), port, port))
        }
        2 => {
            let host: u16 = parts[0].parse().ok()?;
            let container: u16 = parts[1].parse().ok()?;
            Some(("127.0.0.1".to_string(), host, container))
        }
        _ => {
            let ip = parts[0].to_string();
            let host: u16 = parts[parts.len() - 2].parse().ok()?;
            let container: u16 = parts[parts.len() - 1].parse().ok()?;
            let ip = if ip.is_empty() {
                "127.0.0.1".to_string()
            } else {
                ip
            };
            Some((ip, host, container))
        }
    }
}

struct VolumeSpec {
    source: String,
    target: String,
    mode: Option<String>,
}

fn parse_volume_spec(volume: &str) -> Option<VolumeSpec> {
    let parts = volume.split(':').collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    let (source, target, mode) = if looks_like_windows_drive(parts[0], parts.get(1).copied()) {
        (
            format!("{}:{}", parts[0], parts[1]),
            parts.get(2)?.to_string(),
            parts.get(3).map(|value| value.to_string()),
        )
    } else {
        (
            parts[0].to_string(),
            parts[1].to_string(),
            parts.get(2).map(|value| value.to_string()),
        )
    };
    Some(VolumeSpec {
        source,
        target,
        mode,
    })
}

fn looks_like_windows_drive(first: &str, second: Option<&str>) -> bool {
    first.len() == 1
        && first.chars().all(|ch| ch.is_ascii_alphabetic())
        && second
            .map(|value| value.starts_with('\\') || value.starts_with('/'))
            .unwrap_or(false)
}

fn is_bind_source(source: &str) -> bool {
    source.starts_with('.')
        || source.starts_with('/')
        || source.starts_with('\\')
        || looks_like_windows_drive(
            source.split(':').next().unwrap_or_default(),
            source.split_once(':').map(|(_, rest)| rest),
        )
}

fn local_bind_source(root: &Path, source: &str) -> PathBuf {
    let normalized = source.replace('/', "\\");
    let path = PathBuf::from(&normalized);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

/// Generate a nerdctl command string for SSH execution.
pub fn nerdctl_command(cfg: &HyperVConfig, args: &[impl AsRef<str>]) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push("sudo".to_string());
    parts.push("nerdctl".to_string());
    if !cfg.namespace.is_empty() {
        parts.extend(["--namespace".to_string(), shell_quote(&cfg.namespace)]);
    }
    parts.extend(args.iter().map(|a| shell_quote(a.as_ref())));
    parts.join(" ")
}

pub fn validate_config_for_shell(cfg: &HyperVConfig) -> Result<(), HyperVError> {
    validate_namespace(&cfg.namespace)?;
    validate_ssh_host(&cfg.ssh_host)?;
    Ok(())
}

fn validate_namespace(namespace: &str) -> Result<(), HyperVError> {
    if namespace.is_empty()
        || namespace
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
    {
        Ok(())
    } else {
        Err(HyperVError::Config(format!(
            "invalid Hyper-V namespace '{}': allowed characters are letters, digits, underscore, dot, and dash",
            namespace
        )))
    }
}

fn validate_ssh_host(host: &str) -> Result<(), HyperVError> {
    let host = host.trim();
    if host.is_empty() {
        return Err(HyperVError::Config("ssh_host must not be empty".into()));
    }
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    let labels: Vec<&str> = host.split('.').collect();
    let valid = labels.iter().all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    if valid {
        Ok(())
    } else {
        Err(HyperVError::Config(format!(
            "invalid Hyper-V ssh_host '{}': expected IP address or DNS hostname",
            host
        )))
    }
}

/// Quote a string for shell usage.
/// Quote a string for POSIX shell usage.
pub fn shell_quote(value: &str) -> String {
    if value.is_empty()
        || value
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/')
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

/// Generate a project slug.
pub fn project_slug(value: &str) -> String {
    let re = regex::Regex::new(r"[^a-z0-9_.-]+").unwrap();
    let slug = re.replace_all(&value.to_lowercase(), "-").to_string();
    let trimmed = slug.trim_matches('-');
    trimmed.chars().take(80).collect::<String>()
}

fn ps_quote(value: &str) -> String {
    value.replace('\'', "''")
}

/// PowerShell that builds the cloud-init NoCloud seed ISO. Prefers `oscdimg`,
/// then `genisoimage`; otherwise falls back to the built-in Windows IMAPI2 COM
/// (`IMAPI2FS.MsftFileSystemImage`) so no external tool is required. Produces a
/// `cidata`-labeled ISO9660+Joliet image with `user-data`/`meta-data` at root.
/// Placeholders `@@SEED@@` and `@@ISO@@` are substituted by the caller.
const ISO_BUILD_SEED_PS: &str = r##"
$ErrorActionPreference = 'Stop'
$seed = '@@SEED@@'
$iso = '@@ISO@@'
if (Test-Path $iso) { Remove-Item -Force $iso }
$oscdimg = Get-Command oscdimg.exe -ErrorAction SilentlyContinue
$geniso = Get-Command genisoimage.exe -ErrorAction SilentlyContinue
if ($oscdimg) {
  & $oscdimg.Source -o -m -j2 -lCIDATA $seed $iso | Out-Null
} elseif ($geniso) {
  & $geniso.Source -output $iso -volid cidata -joliet -rock "$seed\user-data" "$seed\meta-data"
} else {
  if (-not ('PystackIso' -as [type])) {
    Add-Type -Language CSharp -TypeDefinition '
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
public static class PystackIso {
  public static void Write(object comStream, string path) {
    IStream s = (IStream)comStream;
    byte[] buf = new byte[1048576];
    IntPtr pcb = Marshal.AllocHGlobal(sizeof(int));
    try {
      using (var fs = new FileStream(path, FileMode.Create, FileAccess.Write)) {
        while (true) {
          s.Read(buf, buf.Length, pcb);
          int read = Marshal.ReadInt32(pcb);
          if (read <= 0) break;
          fs.Write(buf, 0, read);
        }
      }
    } finally { Marshal.FreeHGlobal(pcb); }
  }
}'
  }
  $fsi = New-Object -ComObject IMAPI2FS.MsftFileSystemImage
  $fsi.FileSystemsToCreate = 3
  $fsi.VolumeName = 'cidata'
  [void]$fsi.Root.AddTree($seed, $false)
  [PystackIso]::Write($fsi.CreateResultImage().ImageStream, $iso)
}
"##;

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_name() {
        assert_eq!(container_name("myapp", "web"), "pystack-myapp-web");
        assert_eq!(
            container_name("My-App", "Web Server"),
            "pystack-my-app-web-server"
        );
    }

    #[test]
    fn test_project_network_name() {
        assert_eq!(project_network_name("myapp"), "pystack-myapp");
        assert_eq!(
            project_network_name("Smart Surveillance SaaS MVP"),
            "pystack-smart-surveillance-saas-mvp"
        );
        assert_eq!(project_network_name("---"), "");
    }

    #[test]
    fn test_service_hostname() {
        assert_eq!(service_hostname("postgres"), "postgres");
        assert_eq!(service_hostname("API Server"), "api-server");
        assert_eq!(service_hostname("---"), "service");
    }

    #[test]
    fn test_parse_port() {
        assert_eq!(
            parse_port("8080:80"),
            Some(("127.0.0.1".to_string(), 8080, 80))
        );
        assert_eq!(parse_port("80"), Some(("127.0.0.1".to_string(), 80, 80)));
        assert_eq!(
            parse_port("127.0.0.1:8080:80"),
            Some(("127.0.0.1".to_string(), 8080, 80))
        );
        assert_eq!(
            parse_port("0.0.0.0:8080:80"),
            Some(("0.0.0.0".to_string(), 8080, 80))
        );
        assert_eq!(
            parse_port("8080:80/tcp"),
            Some(("127.0.0.1".to_string(), 8080, 80))
        );
        assert_eq!(parse_port(""), None);
        assert_eq!(
            parse_port("\"8080:80\""),
            Some(("127.0.0.1".to_string(), 8080, 80))
        );
    }

    #[test]
    fn test_project_slug() {
        assert_eq!(project_slug("My Project"), "my-project");
        assert_eq!(project_slug("test"), "test");
        assert_eq!(project_slug("---"), "");
    }

    #[test]
    fn test_load_save_config() {
        let dir = std::env::temp_dir().join("pystack_hyperv_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Temporarily point config to test dir
        let cfg = HyperVConfig::default();
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        assert!(!json.is_empty());

        let parsed: HyperVConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.vm_name, "stackdeck-linux");
        assert_eq!(parsed.ssh_port, 22);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_shell_quote() {
        assert_eq!(shell_quote("hello"), "hello");
        assert_eq!(shell_quote("hello world"), "'hello world'");
        assert_eq!(shell_quote(""), "");
    }

    #[test]
    fn test_nerdctl_command() {
        let cfg = HyperVConfig::default();
        let cmd = nerdctl_command(&cfg, &["ps", "-a"]);
        assert!(cmd.contains("sudo"));
        assert!(cmd.contains("nerdctl"));
        assert!(cmd.contains("pystack"));
        assert!(cmd.contains("ps"));
    }

    #[test]
    fn test_nerdctl_command_quotes_namespace() {
        let cfg = HyperVConfig {
            namespace: "pystack; touch /tmp/pwn".into(),
            ..HyperVConfig::default()
        };
        let cmd = nerdctl_command(&cfg, &["ps"]);
        assert!(cmd.contains("--namespace 'pystack; touch /tmp/pwn'"));
    }

    #[test]
    fn test_validate_config_rejects_unsafe_shell_fields() {
        let cfg = HyperVConfig {
            namespace: "pystack;touch".into(),
            ssh_host: "127.0.0.1".into(),
            ..HyperVConfig::default()
        };
        assert!(validate_config_for_shell(&cfg).is_err());

        let cfg = HyperVConfig {
            namespace: "pystack".into(),
            ssh_host: "127.0.0.1; calc".into(),
            ..HyperVConfig::default()
        };
        assert!(validate_config_for_shell(&cfg).is_err());

        let cfg = HyperVConfig {
            namespace: "pystack".into(),
            ssh_host: "vm-host.local".into(),
            ..HyperVConfig::default()
        };
        assert!(validate_config_for_shell(&cfg).is_ok());
    }

    #[test]
    fn test_ssh_base_uses_noninteractive_options() {
        let cfg = HyperVConfig {
            ssh_host: "127.0.0.1".into(),
            ..HyperVConfig::default()
        };
        let manager = HyperVManager::new(cfg);
        let args = manager.ssh_base();

        assert!(has_ssh_option(&args, "StrictHostKeyChecking=accept-new"));
        assert!(has_ssh_option(&args, "BatchMode=yes"));
        assert!(has_ssh_option(&args, "NumberOfPasswordPrompts=0"));
    }

    #[test]
    fn test_run_cmd_timeout_returns_error() {
        let manager = HyperVManager::new(HyperVConfig::default());
        let args = sleep_command_args();
        let started = std::time::Instant::now();

        let err = manager.run_cmd(&args, true, Some(1)).unwrap_err();

        assert!(matches!(err, HyperVError::Ssh(msg) if msg.contains("timed out")));
        assert!(started.elapsed() < std::time::Duration::from_secs(4));
    }

    #[cfg(windows)]
    #[test]
    fn test_ps_timeout_returns_powershell_error() {
        let manager = HyperVManager::new(HyperVConfig::default());

        let err = manager
            .ps("Start-Sleep -Seconds 5; 'done'", true, Some(1))
            .unwrap_err();

        assert!(matches!(err, HyperVError::PowerShell(msg) if msg.contains("timed out")));
    }

    #[test]
    fn test_hyperverror_display() {
        let err = HyperVError::Config("test error".into());
        assert!(err.to_string().contains("test error"));
        let err = HyperVError::PowerShell("ps failed".into());
        assert!(err.to_string().contains("ps failed"));
    }

    fn has_ssh_option(args: &[String], option: &str) -> bool {
        args.windows(2)
            .any(|window| window[0] == "-o" && window[1] == option)
    }

    #[cfg(windows)]
    fn sleep_command_args() -> Vec<String> {
        vec![
            "powershell".into(),
            "-NoProfile".into(),
            "-Command".into(),
            "Start-Sleep -Seconds 5; 'done'".into(),
        ]
    }

    #[cfg(not(windows))]
    fn sleep_command_args() -> Vec<String> {
        vec!["sh".into(), "-c".into(), "sleep 5; printf done".into()]
    }
}
