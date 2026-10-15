//! Hyper-V backend for StackDeck Runner.
//!
//! Replaces `hyperv_backend.py` — VM lifecycle, SSH orchestration,
//! container management, ISO generation, port proxy, SMB shares.

pub use stackdeck_types::{HyperVConfig, HyperVService};

/// Smallest supported dynamically expanding VM disk.  Eight GiB is enough for
/// the Debian guest plus the container runtime, while the configured default
/// remains ten GiB.
const MIN_VM_DISK_GB: u32 = 8;

const VM_CREATION_SAFETY_GUARD: &str = r#"
# A stale VM registry or missing settings association MUST block creation.
# Never interpret a failed VM enumeration as an absent VM.
$cimRegistered = @(Get-CimInstance -Namespace 'root\virtualization\v2' -ClassName Msvm_ComputerSystem -ErrorAction Stop |
    Where-Object { $_.Caption -eq 'Virtual Machine' -and $_.ElementName -eq $name })
if ($cimRegistered.Count -gt 0) {
    throw "VM '$name' is registered. Refusing to overwrite or remove an existing VM."
}
$inventory = @(Get-VM -ErrorAction Stop)
if (@($inventory | Where-Object { $_.Name -eq $name }).Count -gt 0) {
    throw "VM '$name' already exists. Refusing to overwrite or remove it."
}
if (Test-Path -LiteralPath $disk -PathType Leaf) {
    throw "The VM disk '$disk' already exists. Refusing to overwrite it."
}
foreach ($artifact in @((Join-Path $root 'seed.iso'), (Join-Path $root 'seed.vhdx'))) {
    if (Test-Path -LiteralPath $artifact -PathType Leaf) {
        throw "Cloud-init artifact '$artifact' already exists; refusing to overwrite it."
    }
}
if (Test-Path -LiteralPath (Join-Path $root 'seed') -PathType Container) {
    throw "Cloud-init seed directory already exists under '$root'; refusing to reuse it."
}
"#;

/// Recheck VM registry and target VM disk *after* this same initialization
/// generated its cloud-init seed. Do not reject our just-created seed media.
const VM_FINAL_CREATION_SAFETY_GUARD: &str = r#"
$cimRegistered = @(Get-CimInstance -Namespace 'root\virtualization\v2' -ClassName Msvm_ComputerSystem -ErrorAction Stop |
    Where-Object { $_.Caption -eq 'Virtual Machine' -and $_.ElementName -eq $name })
if ($cimRegistered.Count -gt 0) {
    throw "VM '$name' is registered. Refusing to overwrite or remove an existing VM."
}
$inventory = @(Get-VM -ErrorAction Stop)
if (@($inventory | Where-Object { $_.Name -eq $name }).Count -gt 0) {
    throw "VM '$name' already exists. Refusing to overwrite or remove it."
}
if (Test-Path -LiteralPath $disk -PathType Leaf) {
    throw "The VM disk '$disk' already exists. Refusing to overwrite it."
}
"#;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

// Host-side commands must have a bounded lifetime. A reachable but stuck SSH
// session must not hold a CLI or Docker API request indefinitely.
const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 900;
const BUILD_COMMAND_TIMEOUT_SECS: u64 = 1_800;
const STREAM_PUMP_CAPACITY: usize = 8;
const MAX_CAPTURED_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RuntimeReleaseLock {
    pub schema: String,
    pub image_url: String,
    pub image_sha512: String,
    pub debian_snapshot_timestamp: String,
    pub nerdctl_version: String,
    pub nerdctl_sha256_amd64: String,
}

pub fn release_runtime_lock() -> Result<RuntimeReleaseLock, HyperVError> {
    let lock: RuntimeReleaseLock =
        serde_json::from_str(include_str!("../../../runtime/windows-runtime-lock.json"))?;
    if lock.schema != "https://stackdeck.dev/schemas/windows-runtime-lock-v1.json" {
        return Err(HyperVError::Config(
            "unsupported runtime lock schema".into(),
        ));
    }
    let image_url = lock.image_url.trim();
    if !image_url.starts_with("https://")
        || image_url.contains("/latest/")
        || image_url == "UNLOCKED"
    {
        return Err(HyperVError::Config(
            "runtime image lock is not release-pinned; run scripts/windows/12-Lock-Runtime-Image.ps1".into(),
        ));
    }
    if lock.image_sha512.len() != 128 || !lock.image_sha512.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(HyperVError::Config(
            "runtime image SHA-512 lock is invalid".into(),
        ));
    }
    let stamp = lock.debian_snapshot_timestamp.as_bytes();
    if stamp.len() != 16
        || stamp[8] != b'T'
        || stamp[15] != b'Z'
        || !stamp[..8]
            .iter()
            .chain(stamp[9..15].iter())
            .all(u8::is_ascii_digit)
    {
        return Err(HyperVError::Config(
            "Debian snapshot timestamp must be YYYYMMDDTHHMMSSZ".into(),
        ));
    }
    if lock.nerdctl_version.trim().is_empty()
        || !lock
            .nerdctl_version
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.')
    {
        return Err(HyperVError::Config(
            "nerdctl version lock is invalid".into(),
        ));
    }
    if lock.nerdctl_sha256_amd64.len() != 64
        || !lock
            .nerdctl_sha256_amd64
            .chars()
            .all(|c| c.is_ascii_hexdigit())
    {
        return Err(HyperVError::Config(
            "nerdctl amd64 SHA-256 lock is invalid".into(),
        ));
    }
    Ok(lock)
}

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

struct SeedMedia {
    iso: PathBuf,
    disk: PathBuf,
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

    /// Load the Hyper-V config from disk. SMB credentials are stored outside
    /// the JSON config in a machine-scope DPAPI-protected blob on Windows.
    pub fn load_config() -> Result<HyperVConfig, HyperVError> {
        let path = stackdeck_types::hyperv_config_file();
        if !path.exists() {
            let cfg = HyperVConfig::default();
            Self::save_config(&cfg)?;
            return Ok(cfg);
        }
        let text = std::fs::read_to_string(&path)?;
        let mut cfg = parse_hyperv_config(&text)?;

        // Migrate legacy plaintext JSON credentials once, then rewrite the
        // JSON without smb_password because HyperVConfig skips serialization.
        if !cfg.smb_password.is_empty() {
            Self::save_smb_password_secret(&cfg.smb_password)?;
            let legacy_password = cfg.smb_password.clone();
            let json = serde_json::to_string_pretty(&cfg)?;
            stackdeck_types::atomic_write(&path, json.as_bytes())?;
            cfg.smb_password = legacy_password;
        } else if let Some(secret) = Self::load_smb_password_secret()? {
            cfg.smb_password = secret;
        }
        Ok(cfg)
    }

    /// Save non-secret config as JSON and the optional SMB password in a
    /// separate Windows machine-scope DPAPI-protected blob.
    pub fn save_config(config: &HyperVConfig) -> Result<(), HyperVError> {
        let path = stackdeck_types::hyperv_config_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if !config.smb_password.is_empty() {
            Self::save_smb_password_secret(&config.smb_password)?;
        }
        let json = serde_json::to_string_pretty(config)?;
        stackdeck_types::atomic_write(&path, json.as_bytes())?;
        Ok(())
    }

    fn smb_password_secret_file() -> PathBuf {
        stackdeck_types::hyperv_config_file().with_file_name("smb-password.dpapi")
    }

    #[cfg(target_os = "windows")]
    fn save_smb_password_secret(secret: &str) -> Result<(), HyperVError> {
        let script = r#"$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Security; $plain=[Console]::In.ReadToEnd(); $bytes=[Text.Encoding]::UTF8.GetBytes($plain); $cipher=[Security.Cryptography.ProtectedData]::Protect($bytes,$null,[Security.Cryptography.DataProtectionScope]::LocalMachine); [Console]::Out.Write([Convert]::ToBase64String($cipher))"#;
        let mut child = create_command("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                script,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(secret.as_bytes())?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(HyperVError::Config(format!(
                "Windows DPAPI protection failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let encrypted = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if encrypted.is_empty() {
            return Err(HyperVError::Config(
                "Windows DPAPI returned an empty credential blob".into(),
            ));
        }
        let path = Self::smb_password_secret_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        stackdeck_types::atomic_write(&path, encrypted.as_bytes())?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    fn save_smb_password_secret(_secret: &str) -> Result<(), HyperVError> {
        Err(HyperVError::Config(
            "SMB credential persistence is supported only on Windows with DPAPI".into(),
        ))
    }

    #[cfg(target_os = "windows")]
    fn load_smb_password_secret() -> Result<Option<String>, HyperVError> {
        let path = Self::smb_password_secret_file();
        if !path.exists() {
            return Ok(None);
        }
        let encrypted = std::fs::read_to_string(&path)?;
        if encrypted.trim().is_empty() {
            return Ok(None);
        }
        let script = r#"$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Security; $cipher=[Convert]::FromBase64String([Console]::In.ReadToEnd().Trim()); $plain=[Security.Cryptography.ProtectedData]::Unprotect($cipher,$null,[Security.Cryptography.DataProtectionScope]::LocalMachine); [Console]::Out.Write([Text.Encoding]::UTF8.GetString($plain))"#;
        let mut child = create_command("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                script,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(encrypted.as_bytes())?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(HyperVError::Config(format!(
                "Windows DPAPI credential recovery failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(Some(String::from_utf8_lossy(&output.stdout).to_string()))
    }

    #[cfg(not(target_os = "windows"))]
    fn load_smb_password_secret() -> Result<Option<String>, HyperVError> {
        Ok(None)
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

        let effective_timeout = timeout_secs.unwrap_or(DEFAULT_COMMAND_TIMEOUT_SECS);
        let (output, timed_out) = output_with_timeout(&mut cmd, Some(effective_timeout))
            .map_err(|e| HyperVError::PowerShell(format!("failed to execute PowerShell: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if timed_out {
            return Err(HyperVError::PowerShell(timeout_error_message(
                "PowerShell",
                Some(effective_timeout),
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
        self.ssh_stream_with_timeout(command, check, DEFAULT_COMMAND_TIMEOUT_SECS)
    }

    /// Execute a streaming SSH command with a hard local deadline.
    pub fn ssh_stream_with_timeout(
        &self,
        command: &str,
        check: bool,
        timeout_secs: u64,
    ) -> Result<(), HyperVError> {
        let mut args = self.ssh_base();
        args.insert(1, "-t".to_string());
        args.push(command.to_string());
        self.run_cmd_stream(&args, check, timeout_secs)
    }

    /// Execute SSH while forwarding its stdout and stderr chunks to the
    /// caller. This is used by the Docker pull endpoint so progress reaches
    /// the HTTP client while the remote process is still running.
    pub fn ssh_stream_output<F>(
        &self,
        command: &str,
        check: bool,
        timeout_secs: u64,
        mut on_chunk: F,
    ) -> Result<(), HyperVError>
    where
        F: FnMut(CommandOutputStream, Vec<u8>) -> bool,
    {
        validate_config_for_shell(&self.config)?;
        let mut args = self.ssh_base();
        args.push(command.to_string());
        let mut cmd = create_command(&args[0]);
        cmd.args(&args[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| HyperVError::Ssh(format!("command failed: {e}")))?;
        // A bounded synchronous channel connects the pipe readers to the
        // caller. If the HTTP consumer is slow, backpressure now reaches the
        // OS pipe readers instead of accumulating unbounded process output.
        let (sender, receiver) = std::sync::mpsc::sync_channel(STREAM_PUMP_CAPACITY);
        if let Some(stdout) = child.stdout.take() {
            pump_output(stdout, CommandOutputStream::Stdout, sender.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            pump_output(stderr, CommandOutputStream::Stderr, sender.clone());
        }
        drop(sender);

        let deadline = Instant::now() + Duration::from_secs(timeout_secs.max(1));
        let status = loop {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok((stream, chunk)) => {
                    if !on_chunk(stream, chunk) {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(HyperVError::Ssh(
                            "stream consumer disconnected; remote command was cancelled".into(),
                        ));
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(status) = child
                .try_wait()
                .map_err(|e| HyperVError::Ssh(format!("wait failed: {e}")))?
            {
                while let Ok((stream, chunk)) = receiver.try_recv() {
                    if !on_chunk(stream, chunk) {
                        return Err(HyperVError::Ssh(
                            "stream consumer disconnected while draining command output".into(),
                        ));
                    }
                }
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(HyperVError::Ssh(format!(
                    "{} timed out after {} seconds",
                    args[0],
                    timeout_secs.max(1)
                )));
            }
        };
        if check && !status.success() {
            return Err(HyperVError::Ssh(format!("command failed: {}", args[0])));
        }
        Ok(())
    }

    fn run_cmd_stream(
        &self,
        args: &[String],
        check: bool,
        timeout_secs: u64,
    ) -> Result<(), HyperVError> {
        let mut cmd = create_command(&args[0]);
        cmd.args(&args[1..])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let mut child = cmd
            .spawn()
            .map_err(|e| HyperVError::Ssh(format!("command failed: {}", e)))?;
        let deadline = Instant::now() + Duration::from_secs(timeout_secs.max(1));
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|e| HyperVError::Ssh(format!("wait failed: {}", e)))?
            {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(HyperVError::Ssh(format!(
                    "{} timed out after {} seconds",
                    args[0],
                    timeout_secs.max(1)
                )));
            }
            thread::sleep(Duration::from_millis(50));
        };

        if check && !status.success() {
            return Err(HyperVError::Ssh(format!("command failed: {}", args[0])));
        }

        Ok(())
    }

    pub fn ssh(&self, command: &str, check: bool) -> Result<String, HyperVError> {
        validate_config_for_shell(&self.config)?;
        let mut args = self.ssh_base();
        args.push(command.to_string());
        self.run_cmd(&args, check, Some(DEFAULT_COMMAND_TIMEOUT_SECS))
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

        let (output, timed_out) =
            wait_child_output_with_timeout(child, Some(DEFAULT_COMMAND_TIMEOUT_SECS))
                .map_err(|e| HyperVError::Ssh(format!("SSH wait failed: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if timed_out {
            return Err(HyperVError::Ssh(timeout_error_message(
                "ssh input command",
                Some(DEFAULT_COMMAND_TIMEOUT_SECS),
                &stdout,
                &stderr,
            )));
        }

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
        self.run_cmd(&args, true, Some(DEFAULT_COMMAND_TIMEOUT_SECS))?;
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
        self.run_cmd(&args, true, Some(DEFAULT_COMMAND_TIMEOUT_SECS))?;
        Ok(())
    }

    /// Authenticate nerdctl using a Docker API registry credential. The
    /// secret is sent over SSH stdin, never interpolated into a guest shell
    /// command or written on the Windows command line.
    pub fn login_registry(
        &self,
        registry: &str,
        username: &str,
        secret: &str,
    ) -> Result<(), HyperVError> {
        let registry = registry.trim();
        if registry.is_empty()
            || !registry
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | ':' | '-'))
        {
            return Err(HyperVError::Config("invalid registry host".to_string()));
        }
        if username.trim().is_empty() || secret.is_empty() {
            return Err(HyperVError::Config(
                "registry credentials require a username and secret".to_string(),
            ));
        }
        let command = nerdctl_command(
            &self.config,
            &[
                "login",
                registry,
                "--username",
                username,
                "--password-stdin",
            ],
        );
        self.ssh_input(&command, &format!("{secret}\n"), true)?;
        Ok(())
    }

    pub fn download_image(
        &self,
        url: &str,
        output: &str,
        sha256: Option<&str>,
        sha512: Option<&str>,
        force: bool,
    ) -> Result<(), HyperVError> {
        if url.trim().is_empty() {
            return Err(HyperVError::Config("image URL must not be empty".into()));
        }
        if sha256.is_none_or(|value| value.trim().is_empty())
            && sha512.is_none_or(|value| value.trim().is_empty())
        {
            return Err(HyperVError::Config(
                "a SHA-256 or SHA-512 digest is required for every downloaded VM image".into(),
            ));
        }
        let output_path = if output.trim().is_empty() {
            let file_name = url
                .rsplit('/')
                .next()
                .filter(|value| !value.is_empty())
                .unwrap_or("debian-cloud.img");
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
            if let Some(expected) = sha512 {
                self.verify_file_sha512(&output_path, expected)?;
            }
            return Ok(());
        }
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let partial_path = PathBuf::from(format!("{}.partial", output_path.to_string_lossy()));
        let script = format!(
            r#"$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$url = '{url}'
$tmp = '{tmp}'
if (Test-Path $tmp) {{ Remove-Item -Force $tmp }}
try {{
  Start-BitsTransfer -Source $url -Destination $tmp -ErrorAction Stop
}} catch {{
  Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing
}}
"#,
            url = ps_quote(url),
            tmp = ps_quote(&partial_path.to_string_lossy()),
        );
        self.ps(&script, true, None)?;
        if let Some(expected) = sha256 {
            if let Err(error) = self.verify_file_sha256(&partial_path, expected) {
                let _ = std::fs::remove_file(&partial_path);
                return Err(error);
            }
        }
        if let Some(expected) = sha512 {
            if let Err(error) = self.verify_file_sha512(&partial_path, expected) {
                let _ = std::fs::remove_file(&partial_path);
                return Err(error);
            }
        }
        self.ps(
            &format!(
                "Move-Item -LiteralPath '{}' -Destination '{}' -Force",
                ps_quote(&partial_path.to_string_lossy()),
                ps_quote(&output_path.to_string_lossy())
            ),
            true,
            Some(60),
        )?;
        Ok(())
    }

    fn ensure_vm_creation_is_safe(&self) -> Result<(), HyperVError> {
        let disk =
            PathBuf::from(&self.config.vm_root).join(format!("{}.vhdx", self.config.vm_name));
        let script = format!(
            "$ErrorActionPreference = 'Stop'\n$name = '{}'\n$disk = '{}'\n$root = '{}'\n{}",
            ps_quote(&self.config.vm_name),
            ps_quote(&disk.to_string_lossy()),
            ps_quote(&self.config.vm_root),
            VM_CREATION_SAFETY_GUARD
        );
        self.ps(&script, true, Some(60))?;
        Ok(())
    }

    pub fn create_vm(&self, iso: &str) -> Result<(), HyperVError> {
        self.ensure_vm_creation_is_safe()?;
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
$configuredMb = {memory_mb}
$os = Get-CimInstance Win32_OperatingSystem
$freeMb = [math]::Floor($os.FreePhysicalMemory / 1024)
$availableMb = $freeMb - 1024
$targetMb = if ($availableMb -lt $configuredMb) {{ $availableMb }} else {{ $configuredMb }}
if ($targetMb -lt 1024) {{ $targetMb = 1024 }}
$targetMb = [int]([math]::Floor($targetMb / 2) * 2)
{vm_safety_guard}
if (!(Test-Path $root)) {{ New-Item -ItemType Directory -Force -Path $root | Out-Null }}
if (!(Test-Path $disk)) {{ New-VHD -Path $disk -Dynamic -SizeBytes {disk_gb}GB | Out-Null }}
New-VM -Name $name -Generation 2 -MemoryStartupBytes ($targetMb * 1MB) -VHDPath $disk -SwitchName $switchName -Path $root | Out-Null
Set-VMProcessor -VMName $name -Count {cpus}
Set-VMMemory -VMName $name -DynamicMemoryEnabled $true -MinimumBytes 1024MB -StartupBytes ($targetMb * 1MB) -MaximumBytes ($configuredMb * 1MB)
Set-VM -Name $name -AutomaticCheckpointsEnabled $false
if ($iso -and (Test-Path $iso)) {{ Add-VMDvdDrive -VMName $name -Path $iso | Out-Null }}
try {{ Set-VMFirmware -VMName $name -EnableSecureBoot On -SecureBootTemplate 'MicrosoftUEFICertificateAuthority' }} catch {{}}
(Get-VM -Name $name).State
"#,
            name = ps_quote(&self.config.vm_name),
            root = ps_quote(&root.to_string_lossy()),
            disk = ps_quote(&disk.to_string_lossy()),
            iso = ps_quote(iso),
            switch = ps_quote(&self.config.switch_name),
            disk_gb = self.config.vm_disk_gb.max(MIN_VM_DISK_GB),
            memory_mb = self.config.vm_memory_mb.max(1024),
            cpus = self.config.vm_cpu_count.max(1),
            vm_safety_guard = VM_CREATION_SAFETY_GUARD,
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
        self.ensure_vm_creation_is_safe()?;
        let public_key = self.ensure_ssh_key()?;
        let mut cfg = self.config.clone();
        cfg.ssh_public_key = public_key.clone();
        if cfg.ssh_identity.is_empty() {
            cfg.ssh_identity = stackdeck_types::registry_dir()
                .join("id_ed25519")
                .to_string_lossy()
                .to_string();
        }
        HyperVManager::save_config(&cfg)?;
        let seed = self.write_cloud_init_seed(&public_key)?;
        let root = PathBuf::from(&self.config.vm_root);
        std::fs::create_dir_all(&root)?;
        let disk = root.join(format!("{}.vhdx", self.config.vm_name));
        let script = format!(
            r#"$ErrorActionPreference = 'Stop'
$name = '{name}'
$root = '{root}'
$src = '{src}'
$disk = '{disk}'
$seedIso = '{seed_iso}'
$seedDisk = '{seed_disk}'
$switchName = '{switch}'
$configuredMb = {memory_mb}
$os = Get-CimInstance Win32_OperatingSystem
$freeMb = [math]::Floor($os.FreePhysicalMemory / 1024)
$availableMb = $freeMb - 1024
$targetMb = if ($availableMb -lt $configuredMb) {{ $availableMb }} else {{ $configuredMb }}
if ($targetMb -lt 1024) {{ $targetMb = 1024 }}
$targetMb = [int]([math]::Floor($targetMb / 2) * 2)
{vm_safety_guard}
if (!(Test-Path $root)) {{ New-Item -ItemType Directory -Force -Path $root | Out-Null }}
[IO.File]::Copy($src, $disk, $false)
attrib -S -C $disk 2>$null
try {{ Resize-VHD -Path $disk -SizeBytes {disk_gb}GB }} catch {{ Write-Warning $_ }}
New-VM -Name $name -Generation 2 -MemoryStartupBytes ($targetMb * 1MB) -VHDPath $disk -SwitchName $switchName -Path $root | Out-Null
Set-VMProcessor -VMName $name -Count {cpus}
Set-VMMemory -VMName $name -DynamicMemoryEnabled $true -MinimumBytes 1024MB -StartupBytes ($targetMb * 1MB) -MaximumBytes ($configuredMb * 1MB)
Set-VM -Name $name -AutomaticCheckpointsEnabled $false
Add-VMDvdDrive -VMName $name -Path $seedIso | Out-Null
Add-VMHardDiskDrive -VMName $name -Path $seedDisk | Out-Null
try {{ Set-VMFirmware -VMName $name -EnableSecureBoot Off }} catch {{}}
if (-not {no_start}) {{ Start-VM -Name $name | Out-Null }}
(Get-VM -Name $name).State
"#,
            name = ps_quote(&self.config.vm_name),
            root = ps_quote(&root.to_string_lossy()),
            src = ps_quote(&image.to_string_lossy()),
            disk = ps_quote(&disk.to_string_lossy()),
            seed_iso = ps_quote(&seed.iso.to_string_lossy()),
            seed_disk = ps_quote(&seed.disk.to_string_lossy()),
            switch = ps_quote(&self.config.switch_name),
            disk_gb = self.config.vm_disk_gb.max(MIN_VM_DISK_GB),
            memory_mb = self.config.vm_memory_mb.max(1024),
            cpus = self.config.vm_cpu_count.max(1),
            vm_safety_guard = VM_FINAL_CREATION_SAFETY_GUARD,
            no_start = if no_start { "$true" } else { "$false" },
        );
        self.ps(&script, true, Some(240))?;
        if discover_ip && !no_start {
            match self.discover_ip(timeout) {
                Ok(ip) => {
                    cfg.ssh_host = ip;
                    HyperVManager::save_config(&cfg)?;
                }
                Err(first_err) => {
                    eprintln!(
                        "initial IP discovery failed; power-cycling VM once and retrying: {}",
                        first_err
                    );
                    let _ = self.vm_stop();
                    thread::sleep(Duration::from_secs(10));
                    self.vm_start()?;
                    let ip = self.discover_ip(timeout)?;
                    cfg.ssh_host = ip;
                    HyperVManager::save_config(&cfg)?;
                }
            }
        }
        Ok(())
    }

    pub fn init_vm(
        &self,
        image_vhdx: &str,
        url: Option<&str>,
        sha256: Option<&str>,
        sha512: Option<&str>,
        timeout: u32,
    ) -> Result<(), HyperVError> {
        self.ensure_vm_creation_is_safe()?;
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
                    .unwrap_or("debian-cloud.img"),
            );
            self.download_image(image_url, &raw.to_string_lossy(), sha256, sha512, false)?;
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
        image_path = self.prepare_cloud_image(&image_path)?;
        self.create_cloud_vm(&image_path.to_string_lossy(), false, true, timeout)?;
        let cfg = HyperVManager::load_config()?;
        let mgr = HyperVManager::new(cfg);
        mgr.wait_for_ssh(timeout)?;
        mgr.bootstrap()?;
        Ok(())
    }

    pub fn ensure_ssh_key(&self) -> Result<String, HyperVError> {
        let mut identity = if self.config.ssh_identity.trim().is_empty() {
            stackdeck_types::registry_dir().join("id_ed25519")
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
        let runtime = release_runtime_lock()?;
        let script = format!(
            r#"set -eu
export DEBIAN_FRONTEND=noninteractive
sudo cloud-init status --wait || true
SNAPSHOT='{snapshot}'
NERDCTL_VERSION='{nerdctl_version}'
NERDCTL_SHA256='{nerdctl_sha256}'
# Use one qualified Debian snapshot instead of live repositories.
sudo rm -f /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources 2>/dev/null || true
printf 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/%s/ bookworm main\n' "$SNAPSHOT" | sudo tee /etc/apt/sources.list >/dev/null
sudo apt-get -o Acquire::Check-Valid-Until=false update -y
sudo apt-get -o Acquire::Check-Valid-Until=false install -y ca-certificates curl gnupg containerd cifs-utils cloud-guest-utils containernetworking-plugins uidmap iptables
sudo growpart /dev/sda 1 || true
sudo resize2fs /dev/sda1 || true
sudo systemctl enable --now containerd
if ! command -v nerdctl >/dev/null 2>&1 || ! command -v buildctl >/dev/null 2>&1 || ! command -v buildkitd >/dev/null 2>&1; then
  ARCH=$(uname -m)
  case "$ARCH" in x86_64|amd64) NARCH=amd64 ;; *) echo "unsupported release guest arch $ARCH" >&2; exit 1 ;; esac
  ARCHIVE="nerdctl-full-${{NERDCTL_VERSION}}-linux-${{NARCH}}.tar.gz"
  curl -fsSL "https://github.com/containerd/nerdctl/releases/download/v${{NERDCTL_VERSION}}/${{ARCHIVE}}" -o "/tmp/${{ARCHIVE}}"
  printf '%s  %s\n' "$NERDCTL_SHA256" "/tmp/${{ARCHIVE}}" | sha256sum -c -
  sudo tar -C /usr/local -xzf "/tmp/${{ARCHIVE}}" bin/nerdctl bin/buildctl bin/buildkitd
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
sudo tee /etc/systemd/system/buildkit.service >/dev/null <<'UNIT'
[Unit]
Description=BuildKit daemon for StackDeck
After=containerd.service
Requires=containerd.service

[Service]
ExecStart=/usr/local/bin/buildkitd --containerd-worker=true
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
UNIT
sudo systemctl daemon-reload
sudo systemctl enable --now buildkit
sudo systemctl is-active --quiet buildkit
buildctl --addr unix:///run/buildkit/buildkitd.sock debug workers >/dev/null
sudo nerdctl --namespace stackdeck version >/dev/null
"#,
            snapshot = runtime.debian_snapshot_timestamp,
            nerdctl_version = runtime.nerdctl_version,
            nerdctl_sha256 = runtime.nerdctl_sha256_amd64.to_ascii_lowercase(),
        );
        let started = Instant::now();
        loop {
            self.wait_for_ssh(60)?;
            match self.ssh_stream(&script, true) {
                Ok(()) => break,
                Err(err) if started.elapsed() < Duration::from_secs(300) => {
                    eprintln!("bootstrap SSH session failed; retrying: {}", err);
                    thread::sleep(Duration::from_secs(10));
                }
                Err(err) => return Err(err),
            }
        }
        self.apply_registry_mirrors()?;
        Ok(())
    }

    fn verify_file_sha256(&self, path: &Path, expected: &str) -> Result<(), HyperVError> {
        let expected = expected.trim().to_ascii_lowercase();
        if expected.is_empty() {
            return Ok(());
        }
        let script = format!(
            r#"$path = '{path}'
if (Get-Command Get-FileHash -ErrorAction SilentlyContinue) {{
  (Get-FileHash -Algorithm SHA256 -Path $path).Hash.ToLowerInvariant()
}} else {{
  $sha = [System.Security.Cryptography.SHA256]::Create()
  $stream = [System.IO.File]::OpenRead($path)
  try {{
    ($sha.ComputeHash($stream) | ForEach-Object {{ $_.ToString('x2') }}) -join ''
  }} finally {{
    $stream.Dispose()
    $sha.Dispose()
  }}
}}"#,
            path = ps_quote(&path.to_string_lossy())
        );
        let actual = self
            .ps(&script, true, Some(120))?
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

    fn verify_file_sha512(&self, path: &Path, expected: &str) -> Result<(), HyperVError> {
        let expected = expected.trim().to_ascii_lowercase();
        if expected.is_empty() {
            return Ok(());
        }
        let script = format!(
            r#"$path = '{path}'
if (Get-Command Get-FileHash -ErrorAction SilentlyContinue) {{
  (Get-FileHash -Algorithm SHA512 -Path $path).Hash.ToLowerInvariant()
}} else {{
  $sha = [System.Security.Cryptography.SHA512]::Create()
  $stream = [System.IO.File]::OpenRead($path)
  try {{
    ($sha.ComputeHash($stream) | ForEach-Object {{ $_.ToString('x2') }}) -join ''
  }} finally {{
    $stream.Dispose()
    $sha.Dispose()
  }}
}}"#,
            path = ps_quote(&path.to_string_lossy())
        );
        let actual = self
            .ps(&script, true, Some(180))?
            .trim()
            .to_ascii_lowercase();
        if actual != expected {
            return Err(HyperVError::Config(format!(
                "SHA512 mismatch for {}: expected {}, got {}",
                path.display(),
                expected,
                actual
            )));
        }
        Ok(())
    }

    fn prepare_cloud_image(&self, source: &Path) -> Result<PathBuf, HyperVError> {
        if has_extension(source, "vhdx") {
            return Ok(source.to_path_buf());
        }
        if has_extension(source, "vhd") {
            return self.convert_vhd_to_vhdx(source);
        }
        if is_tar_archive(source) {
            let extracted = self.extract_cloud_archive(source)?;
            return self.prepare_cloud_image(&extracted);
        }
        self.convert_qemu_to_vhdx(source)
    }

    fn extract_cloud_archive(&self, source: &Path) -> Result<PathBuf, HyperVError> {
        let extract_dir = PathBuf::from(&self.config.vm_root)
            .join("images")
            .join(format!("{}.extracted", archive_stem(source)));
        std::fs::create_dir_all(&extract_dir)?;

        self.run_cmd(
            &[
                "tar".to_string(),
                "-xf".to_string(),
                source.to_string_lossy().to_string(),
                "-C".to_string(),
                extract_dir.to_string_lossy().to_string(),
            ],
            true,
            Some(900),
        )?;

        find_disk_image(&extract_dir).ok_or_else(|| {
            HyperVError::Config(format!(
                "archive {} did not contain a .vhdx or .vhd image",
                source.display()
            ))
        })
    }

    fn convert_vhd_to_vhdx(&self, source: &Path) -> Result<PathBuf, HyperVError> {
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
if (Test-Path -LiteralPath $out -PathType Leaf) {{ throw "Converted image already exists at $out. Refusing to overwrite it." }}
compact.exe /U $src | Out-Null
fsutil sparse setflag $src 0 2>$null | Out-Null
attrib -S -C $src 2>$null
Convert-VHD -Path $src -DestinationPath $out -VHDType Dynamic
compact.exe /U $out | Out-Null
fsutil sparse setflag $out 0 2>$null | Out-Null
attrib -S -C $out 2>$null
Resize-VHD -Path $out -SizeBytes {disk_gb}GB
"#,
            src = ps_quote(&source.to_string_lossy()),
            out = ps_quote(&output.to_string_lossy()),
            disk_gb = self.config.vm_disk_gb.max(MIN_VM_DISK_GB),
        );
        self.ps(&script, true, Some(900))?;
        Ok(output)
    }

    fn convert_qemu_to_vhdx(&self, source: &Path) -> Result<PathBuf, HyperVError> {
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
if (Test-Path -LiteralPath $out -PathType Leaf) {{ throw "Converted image already exists at $out. Refusing to overwrite it." }}
$qemu = Get-Command qemu-img.exe -ErrorAction SilentlyContinue
if (-not $qemu) {{ $qemu = Get-Command qemu-img -ErrorAction SilentlyContinue }}
$qemuPath = if ($qemu) {{ $qemu.Source }} else {{ $null }}
if (-not $qemuPath) {{
  foreach ($candidate in @("$env:ProgramFiles\qemu\qemu-img.exe", "${{env:ProgramFiles(x86)}}\qemu\qemu-img.exe", "$env:ChocolateyInstall\bin\qemu-img.exe")) {{
    if ($candidate -and (Test-Path $candidate)) {{ $qemuPath = $candidate; break }}
  }}
}}
if (-not $qemuPath) {{ throw 'qemu-img is required to convert qcow2/img cloud images to VHDX. Install QEMU/qemu-img, add it to PATH, or provide a pre-converted .vhdx with --image-vhdx.' }}
& $qemuPath convert -O vhdx -o subformat=dynamic $src $out
if ($LASTEXITCODE -ne 0) {{ throw "qemu-img convert failed with exit code $LASTEXITCODE" }}
compact.exe /U $out | Out-Null
fsutil sparse setflag $out 0 2>$null | Out-Null
attrib -S -C $out 2>$null
Resize-VHD -Path $out -SizeBytes {disk_gb}GB
"#,
            src = ps_quote(&source.to_string_lossy()),
            out = ps_quote(&output.to_string_lossy()),
            disk_gb = self.config.vm_disk_gb.max(MIN_VM_DISK_GB),
        );
        self.ps(&script, true, Some(900))?;
        Ok(output)
    }

    fn write_cloud_init_seed(&self, public_key: &str) -> Result<SeedMedia, HyperVError> {
        let seed_dir = PathBuf::from(&self.config.vm_root).join("seed");
        std::fs::create_dir_all(&seed_dir)?;
        let user_data = cloud_init_user_data(&self.config.ssh_user, public_key);
        std::fs::write(seed_dir.join("user-data"), user_data)?;
        std::fs::write(
            seed_dir.join("meta-data"),
            format!(
                "instance-id: {}\nlocal-hostname: {}\n",
                self.config.vm_name, self.config.vm_name
            ),
        )?;
        std::fs::write(
            seed_dir.join("ovf-env.xml"),
            azure_ovf_env(&self.config.vm_name, &self.config.ssh_user, public_key),
        )?;
        let iso = PathBuf::from(&self.config.vm_root).join("seed.iso");
        let script = ISO_BUILD_SEED_PS
            .replace("@@SEED@@", &ps_quote(&seed_dir.to_string_lossy()))
            .replace("@@ISO@@", &ps_quote(&iso.to_string_lossy()));
        self.ps(&script, true, Some(180))?;
        let disk = PathBuf::from(&self.config.vm_root).join("seed.vhdx");
        self.write_cloud_init_seed_disk(&seed_dir, &disk)?;
        Ok(SeedMedia { iso, disk })
    }

    fn write_cloud_init_seed_disk(&self, seed_dir: &Path, disk: &Path) -> Result<(), HyperVError> {
        let script = format!(
            r#"$ErrorActionPreference = 'Stop'
$seed = '{seed}'
$vhd = '{vhd}'
if (Test-Path -LiteralPath $vhd -PathType Leaf) {{ throw "Seed VHDX already exists at $vhd; refusing to overwrite it." }}
New-VHD -Path $vhd -SizeBytes 64MB -Dynamic | Out-Null
$mounted = Mount-VHD -Path $vhd -Passthru
try {{
  $disk = Get-Disk -Number $mounted.DiskNumber
  if ($disk.PartitionStyle -eq 'RAW') {{
    Initialize-Disk -Number $disk.Number -PartitionStyle MBR | Out-Null
  }}
  $partition = New-Partition -DiskNumber $disk.Number -UseMaximumSize -AssignDriveLetter
  Format-Volume -Partition $partition -FileSystem FAT32 -NewFileSystemLabel CIDATA -Confirm:$false | Out-Null
  $volume = Get-Volume -Partition $partition
  $drive = "$($volume.DriveLetter):\"
  Copy-Item -Force -Path (Join-Path $seed '*') -Destination $drive
}} finally {{
  Dismount-VHD -Path $vhd
}}
"#,
            seed = ps_quote(&seed_dir.to_string_lossy()),
            vhd = ps_quote(&disk.to_string_lossy()),
        );
        self.ps(&script, true, Some(180))?;
        Ok(())
    }

    pub fn discover_ip(&self, timeout: u32) -> Result<String, HyperVError> {
        let started = Instant::now();
        loop {
            let ip = self.vm_ip()?.trim().to_string();
            if !ip.is_empty() && !ip.starts_with("169.254.") {
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
        let mut cfg = self.config.clone();
        let mut consecutive_successes = 0;
        loop {
            if let Ok(ip) = self.vm_ip() {
                let ip = ip.trim();
                if !ip.is_empty() && !ip.starts_with("169.254.") && ip != cfg.ssh_host {
                    validate_ssh_host(ip)?;
                    cfg.ssh_host = ip.to_string();
                    HyperVManager::save_config(&cfg)?;
                    consecutive_successes = 0;
                }
            }
            if !cfg.ssh_host.trim().is_empty() {
                if HyperVManager::new(cfg.clone()).ssh_probe(3) {
                    consecutive_successes += 1;
                    if consecutive_successes >= 2 {
                        return Ok(());
                    }
                } else {
                    consecutive_successes = 0;
                }
            }
            if started.elapsed() > Duration::from_secs(timeout.max(1) as u64) {
                return Err(HyperVError::Ssh(format!(
                    "SSH did not become reachable at {}:{} within {}s",
                    cfg.ssh_host, cfg.ssh_port, timeout
                )));
            }
            thread::sleep(Duration::from_secs(2));
        }
    }

    pub fn repair(&self, timeout: u32) -> Result<(), HyperVError> {
        if self.vm_state().unwrap_or_default().trim() != "Running" {
            self.vm_start()?;
        }
        let mut cfg = self.config.clone();
        if cfg.ssh_host.trim().is_empty() {
            cfg.ssh_host = self.discover_ip(timeout)?;
            HyperVManager::save_config(&cfg)?;
        }
        let mgr = HyperVManager::new(cfg);
        mgr.wait_for_ssh(timeout)?;
        mgr.bootstrap()?;
        mgr.apply_registry_mirrors()?;
        Ok(())
    }

    pub fn apply_registry_mirrors(&self) -> Result<(), HyperVError> {
        if self.config.registry_mirrors.is_empty() {
            return Ok(());
        }
        for (registry, endpoints) in &self.config.registry_mirrors {
            validate_registry_mirror(registry, endpoints)?;
        }
        let json = serde_json::to_string(&self.config.registry_mirrors)?;
        let script = format!(
            r#"set -eu
sudo mkdir -p /etc/containerd/certs.d
cat > /tmp/stackdeck_mirrors.json <<'JSON'
{json}
JSON
sudo python3 - <<'PY'
import json, os
with open('/tmp/stackdeck_mirrors.json', 'r', encoding='utf-8') as fh:
    mirrors = json.load(fh)
for registry, endpoints in mirrors.items():
    d = '/etc/containerd/certs.d/' + registry
    os.makedirs(d, mode=0o755, exist_ok=True)
    with open(os.path.join(d, 'hosts.toml'), 'w', encoding='utf-8') as f:
        for endpoint in endpoints:
            f.write('[host."%s"]\n  capabilities = ["pull", "resolve"]\n' % endpoint)
    os.chmod(os.path.join(d, 'hosts.toml'), 0o644)
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
        validate_registry_mirror(registry, endpoints)?;
        let mut cfg = self.config.clone();
        cfg.registry_mirrors
            .insert(registry.to_string(), endpoints.to_vec());
        HyperVManager::save_config(&cfg)?;
        HyperVManager::new(cfg).apply_registry_mirrors()?;
        Ok(())
    }

    pub fn mirror_remove(&self, registry: &str) -> Result<(), HyperVError> {
        validate_registry_name(registry)?;
        let mut cfg = self.config.clone();
        let registry = registry.trim();
        // Apply the removal to the guest before committing local state. If
        // SSH or containerd restart fails, retain the JSON entry so the CLI
        // does not report a mirror as removed while it remains active.
        let guest_dir = format!("/etc/containerd/certs.d/{registry}");
        self.ssh(
            &format!(
                "sudo rm -rf -- {} && sudo systemctl restart containerd",
                shell_quote(&guest_dir)
            ),
            true,
        )?;
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
            .unwrap_or_else(|| format!("stackdeck_{}", unix_timestamp()));
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
        validate_windows_host(&self.config.windows_host)?;
        let share = project_slug(name);
        let remote = format!("/mnt/stackdeck-shares/{}", share);
        let cred = format!("/etc/stackdeck/cifs-{}.cred", share);
        let unc = format!("//{}/{}", self.config.windows_host, name);
        self.install_smb_credentials(&cred)?;
        let cmd = format!(
            "sudo mkdir -p {remote} && if ! mountpoint -q {remote}; then sudo mount -t cifs {unc} {remote} -o credentials={cred},vers=3.0,iocharset=utf8,noserverino; fi; echo {remote}",
            remote = shell_quote(&remote),
            cred = shell_quote(&cred),
            unc = shell_quote(&unc),
        );
        self.ssh(&cmd, true)
    }

    fn install_smb_credentials(&self, destination: &str) -> Result<(), HyperVError> {
        let local = std::env::temp_dir().join(format!(
            "stackdeck-cifs-{}-{}.cred",
            std::process::id(),
            unix_timestamp()
        ));
        let contents = format!(
            "username={}\npassword={}\n",
            self.config.smb_user, self.config.smb_password
        );
        stackdeck_types::atomic_write(&local, contents.as_bytes())?;
        let remote_tmp = format!(
            "/tmp/stackdeck-cifs-{}-{}.cred",
            std::process::id(),
            unix_timestamp()
        );
        let copy_result = self.scp_to_vm(&local, &remote_tmp);
        let _ = std::fs::remove_file(&local);
        copy_result?;
        self.ssh(
            &format!(
                "sudo install -d -m 0700 {parent} && sudo install -m 0600 {source} {destination} && rm -f {source}",
                parent = shell_quote(
                    Path::new(destination)
                        .parent()
                        .and_then(|path| path.to_str())
                        .unwrap_or("/etc/stackdeck")
                ),
                source = shell_quote(&remote_tmp),
                destination = shell_quote(destination),
            ),
            true,
        )?;
        Ok(())
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
  $neighborIp = Get-NetNeighbor -AddressFamily IPv4 -ErrorAction SilentlyContinue |
    Where-Object {{ $_.LinkLayerAddress -and $_.LinkLayerAddress.ToUpper() -eq $normalized -and $_.IPAddress -match '^\d+\.\d+\.\d+\.\d+$' }} |
    Select-Object -First 1 -ExpandProperty IPAddress
  if ($neighborIp) {{
    $neighborIp
    exit 0
  }}
}}
$dnsName = "$name.mshome.net"
$dnsIp = Resolve-DnsName $dnsName -Type A -ErrorAction SilentlyContinue |
  Where-Object {{ $_.IPAddress -match '^\d+\.\d+\.\d+\.\d+$' }} |
  Select-Object -First 1 -ExpandProperty IPAddress
if ($dnsIp) {{
  $dnsIp
  exit 0
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
        self.run_cmd(&args, true, Some(timeout_secs + 2)).is_ok()
    }

    // -----------------------------------------------------------------------
    // Container operations
    // -----------------------------------------------------------------------

    /// Start a service container on the Hyper-V VM.

    /// Build a container image natively on the Hyper-V VM
    pub fn build_image(
        &self,
        svc: &stackdeck_types::HyperVService,
        tag: &str,
    ) -> Result<(), HyperVError> {
        let Some(build_val) = &svc.build else {
            return Ok(());
        };

        let mut context = svc.root.clone();
        let mut dockerfile = None;
        let mut target = None;
        let mut args_map = std::collections::HashMap::new();
        let mut cache_from = Vec::new();
        let mut cache_to = Vec::new();

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
                    let value = v
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| std::env::var(k).ok());
                    let Some(value) = value else {
                        return Err(HyperVError::Config(format!(
                            "build argument '{k}' has no value and is not set in the environment"
                        )));
                    };
                    args_map.insert(k.clone(), value);
                }
            }
            cache_from = b_obj
                .get("cache_from")
                .and_then(|value| value.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            cache_to = b_obj
                .get("cache_to")
                .and_then(|value| value.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
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

        // Preserve .dockerignore in the transferred staging directory and let
        // BuildKit apply the complete ordered rule set, including `!` negation.
        // Host-side tar exclusions are limited to StackDeck metadata.
        let excludes = vec![
            ".stackdeck".to_string(),
            ".stackdeck-*".to_string(),
            "__pycache__".to_string(),
            ".git".to_string(),
        ];

        let remote_dir = format!(
            "/tmp/stackdeck-build-{}",
            container_name(&svc.project, &svc.name)
        );
        let temp_dir = std::env::temp_dir().join(format!(
            "stackdeck-build-{}",
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
        for cache in cache_from {
            nerdctl_args.extend(["--cache-from".to_string(), cache]);
        }
        for cache in cache_to {
            nerdctl_args.extend(["--cache-to".to_string(), cache]);
        }
        nerdctl_args.push("-t".to_string());
        nerdctl_args.push(tag.to_string());
        nerdctl_args.push(".".to_string());

        let cmd = nerdctl_command(&self.config, &nerdctl_args);
        println!("Building image {} for {}...", tag, svc.name);
        let build_result = self.ssh_stream_with_timeout(
            &format!("cd {} && {}", shell_quote(&remote_dir), cmd),
            true,
            BUILD_COMMAND_TIMEOUT_SECS,
        );
        let cleanup_result = self.ssh(&format!("rm -rf {}", shell_quote(&remote_dir)), false);
        build_result?;
        cleanup_result?;

        Ok(())
    }

    pub fn start_service(
        &self,
        svc: &stackdeck_types::HyperVService,
        build: bool,
    ) -> Result<String, HyperVError> {
        self.start_service_inner(svc, build, true)
    }

    /// Start the guest container while deferring host portproxy activation.
    /// Docker API create/start uses this to hold bound host sockets until the
    /// guest is ready, then releases them immediately before installing the
    /// Windows forwarding rules.
    pub fn start_service_deferred_portproxy(
        &self,
        svc: &stackdeck_types::HyperVService,
        build: bool,
    ) -> Result<String, HyperVError> {
        self.start_service_inner(svc, build, false)
    }

    fn start_service_inner(
        &self,
        svc: &stackdeck_types::HyperVService,
        build: bool,
        activate_portproxy: bool,
    ) -> Result<String, HyperVError> {
        validate_config_for_shell(&self.config)?;
        let cfg = &self.config;
        if cfg.ssh_host.is_empty() {
            return Err(HyperVError::Config(
                "Hyper-V backend is not configured with ssh_host".into(),
            ));
        }

        if !self.ssh_probe(1) {
            println!(
                "VM is offline or unreachable. Starting VM and refreshing its DHCP address..."
            );
            if !self.vm_state()?.eq_ignore_ascii_case("running") {
                self.vm_start()?;
            }
            let refreshed_ip = self.discover_ip(120)?;
            if refreshed_ip != self.config.ssh_host {
                let mut refreshed = self.config.clone();
                refreshed.ssh_host = refreshed_ip;
                Self::save_config(&refreshed)?;
                return Self::new(refreshed).start_service_inner(svc, build, activate_portproxy);
            }
            if !self.ssh_probe(5) {
                return Err(HyperVError::Ssh(
                    "VM is running but SSH is unreachable after DHCP address refresh".into(),
                ));
            }
        }

        let cname = container_name(&svc.project, &svc.name);
        let image = if svc.image.is_empty() {
            format!("stackdeck/{}-{}:latest", svc.project, svc.name)
        } else {
            svc.image.clone()
        };

        let desired_config_hash = service_config_hash(svc)?;
        let existing = self
            .ssh(&nerdctl_command(cfg, &["inspect", &cname]), false)
            .unwrap_or_default();
        let replace_existing = !existing.trim().is_empty()
            && (build || container_config_hash(&existing).as_deref() != Some(&desired_config_hash));
        if !existing.trim().is_empty() && !replace_existing {
            if container_is_running(&existing) {
                if activate_portproxy {
                    self.ensure_portproxy(svc)?;
                }
                return Ok(format!("{cname}: already running"));
            }
            let result = self.ssh(&nerdctl_command(cfg, &["start", &cname]), true)?;
            if activate_portproxy {
                self.ensure_portproxy(svc)?;
            }
            return Ok(result);
        }

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
                    self.ssh_stream_with_timeout(
                        &nerdctl_command(cfg, &["pull", &image]),
                        true,
                        DEFAULT_COMMAND_TIMEOUT_SECS,
                    )?;
                }
            }
        }

        let mut networks = if svc.networks.is_empty() {
            vec![project_network_name(&svc.project)]
        } else {
            svc.networks.clone()
        };
        networks.retain(|network| !network.trim().is_empty());
        networks.sort();
        networks.dedup();
        for network in &networks {
            self.ensure_project_network(network)?;
        }
        let network = networks
            .first()
            .cloned()
            .unwrap_or_else(|| project_network_name(&svc.project));

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
            "--network-alias".to_string(),
            svc.name.clone(),
        ];

        let restart = if svc.restart == "always" || svc.restart == "unless-stopped" {
            "always".to_string()
        } else if svc.restart == "on-failure" {
            svc.restart_max_retries
                .filter(|retries| *retries > 0)
                .map(|retries| format!("on-failure:{retries}"))
                .unwrap_or_else(|| "on-failure".to_string())
        } else {
            "no".to_string()
        };
        args.extend(["--restart".to_string(), restart]);
        args.extend([
            "--label".to_string(),
            format!("stackdeck.config-hash={desired_config_hash}"),
        ]);

        if let Some(logging) = svc.logging.as_ref().and_then(|value| value.as_object()) {
            if let Some(driver) = logging.get("driver").and_then(|value| value.as_str()) {
                args.extend(["--log-driver".to_string(), driver.to_string()]);
            }
            if let Some(options) = logging.get("options").and_then(|value| value.as_object()) {
                for (key, value) in options {
                    let value = value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string());
                    args.extend(["--log-opt".to_string(), format!("{key}={value}")]);
                }
            }
        }

        for (key, value) in &svc.env {
            args.extend(["-e".to_string(), format!("{}={}", key, value)]);
        }

        for port in &svc.ports {
            if is_udp_port(port) {
                return Err(HyperVError::Config(format!(
                    "UDP port publishing is not supported by the production MVP TCP portproxy backend: {}",
                    port
                )));
            }
            if let Some(publish) = nerdctl_publish_arg(port) {
                args.extend(["-p".to_string(), publish]);
            }
        }

        let health_command = svc.healthcheck.as_ref().and_then(|hc| {
            if hc.r#type == "none" {
                None
            } else {
                hc.test.as_ref().and_then(compose_health_command)
            }
        });
        let use_native_health = health_command.is_some() && self.supports_nerdctl_health_flags();

        if use_native_health {
            if let Some(hc) = &svc.healthcheck {
                args.extend(nerdctl_health_args(hc));
            }
        } else if health_command.is_some() {
            println!(
                "{}: nerdctl does not support native health flags; using exec-based health checks",
                svc.name
            );
        }

        for volume in &svc.volumes {
            let prepared = self.ensure_volume_mounts(svc, volume)?;
            args.extend(["-v".to_string(), prepared]);
        }
        self.prepare_service_resources(svc, &mut args)?;

        let mut entrypoint_args = Vec::new();
        if let Some(entrypoint) = &svc.entrypoint {
            match entrypoint {
                serde_json::Value::Array(parts) => {
                    if let Some(program) = parts.first().and_then(|value| value.as_str()) {
                        args.extend(["--entrypoint".to_string(), program.to_string()]);
                        entrypoint_args.extend(
                            parts
                                .iter()
                                .skip(1)
                                .filter_map(|value| value.as_str().map(str::to_string)),
                        );
                    }
                }
                serde_json::Value::String(program) if !program.trim().is_empty() => {
                    args.extend(["--entrypoint".to_string(), program.clone()]);
                }
                _ => {}
            }
        }

        args.push(image);
        args.extend(entrypoint_args);

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

        let backup_name =
            replace_existing.then(|| format!("{}-rollback-{}", cname, unix_timestamp()));
        let existing_was_running = container_is_running(&existing);
        if let Some(backup) = backup_name.as_deref() {
            if existing_was_running {
                self.ssh(&nerdctl_command(cfg, &["stop", &cname]), true)?;
            }
            self.ssh(&nerdctl_command(cfg, &["rename", &cname, backup]), true)?;
        }

        let launch_result = (|| -> Result<String, HyperVError> {
            let result = self.ssh(&nerdctl_command(cfg, &args), true)?;
            for network in networks.iter().skip(1) {
                self.ssh(
                    &nerdctl_command(
                        cfg,
                        &["network", "connect", "--alias", &svc.name, network, &cname],
                    ),
                    true,
                )?;
            }

            if let (Some(hc), Some(command)) = (&svc.healthcheck, health_command.as_deref()) {
                println!("{}: waiting for healthcheck...", svc.name);
                if use_native_health {
                    self.wait_for_native_health(&cname, hc)?;
                } else {
                    self.wait_for_exec_health(&cname, hc, command)?;
                }
            }
            if activate_portproxy {
                self.ensure_portproxy(svc)?;
            }
            Ok(result)
        })();

        match (launch_result, backup_name) {
            (Ok(result), Some(backup)) => {
                if let Err(cleanup_error) =
                    self.ssh(&nerdctl_command(cfg, &["rm", "-f", &backup]), true)
                {
                    eprintln!(
                        "{}: replacement is healthy, but rollback container {} could not be removed: {}",
                        svc.name, backup, cleanup_error
                    );
                }
                Ok(result)
            }
            (Err(error), Some(backup)) => {
                let _ = self.ssh(&nerdctl_command(cfg, &["rm", "-f", &cname]), false);
                let restore = self.ssh(&nerdctl_command(cfg, &["rename", &backup, &cname]), true);
                if restore.is_ok() && existing_was_running {
                    let _ = self.ssh(&nerdctl_command(cfg, &["start", &cname]), false);
                    if activate_portproxy {
                        let _ = self.ensure_portproxy(svc);
                    }
                }
                if let Err(restore_error) = restore {
                    return Err(HyperVError::Ssh(format!(
                        "replacement failed ({error}); rollback also failed ({restore_error})"
                    )));
                }
                Err(error)
            }
            (result, None) => result,
        }
    }

    fn prepare_service_resources(
        &self,
        svc: &stackdeck_types::HyperVService,
        args: &mut Vec<String>,
    ) -> Result<(), HyperVError> {
        for (kind, attachments, resources) in [
            ("secret", &svc.secrets, &svc.secret_resources),
            ("config", &svc.configs, &svc.config_resources),
        ] {
            for attachment in attachments {
                let source = attachment
                    .get("source")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| {
                        HyperVError::Config(format!(
                            "{kind} attachment requires a non-empty source"
                        ))
                    })?;
                if source.len() > 80
                    || source.contains("..")
                    || !source
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
                {
                    return Err(HyperVError::Config(format!(
                        "invalid {kind} source name: {source}"
                    )));
                }
                let resource = resources.get(source).ok_or_else(|| {
                    HyperVError::Config(format!(
                        "{kind} attachment references undefined resource: {source}"
                    ))
                })?;
                if resource
                    .get("external")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
                {
                    return Err(HyperVError::Config(format!(
                        "external {kind} resource '{source}' is not materialized by StackDeck"
                    )));
                }

                let target_default = if kind == "secret" {
                    format!("/run/secrets/{source}")
                } else {
                    format!("/{source}")
                };
                let target = attachment
                    .get("target")
                    .and_then(|value| value.as_str())
                    .unwrap_or(&target_default);
                if !target.starts_with('/')
                    || target.contains(':')
                    || target.chars().any(|ch| ch.is_ascii_control())
                {
                    return Err(HyperVError::Config(format!(
                        "invalid {kind} target path: {target}"
                    )));
                }

                let (local, remove_local) =
                    materialize_resource_file(&svc.root, resource, kind, source)?;
                let remote_tmp = format!(
                    "/tmp/stackdeck-{}-{}-{}",
                    project_slug(&svc.project),
                    project_slug(&svc.name),
                    source
                );
                let remote_dir = format!(
                    "/var/lib/stackdeck/resources/{}/{}",
                    project_slug(&svc.project),
                    project_slug(&svc.name)
                );
                let remote_final = format!("{remote_dir}/{source}");
                let copy_result = self.scp_to_vm(&local, &remote_tmp);
                if remove_local {
                    let _ = std::fs::remove_file(&local);
                }
                copy_result?;

                let mode = attachment
                    .get("mode")
                    .and_then(|value| {
                        value
                            .as_str()
                            .map(str::to_string)
                            .or_else(|| value.as_u64().map(|value| format!("{value:o}")))
                    })
                    .unwrap_or_else(|| {
                        if kind == "secret" {
                            "0400".to_string()
                        } else {
                            "0444".to_string()
                        }
                    });
                if mode.len() > 4 || !mode.chars().all(|ch| matches!(ch, '0'..='7')) {
                    return Err(HyperVError::Config(format!(
                        "invalid {kind} mode for {source}: {mode}"
                    )));
                }
                let owner = ["uid", "gid"]
                    .into_iter()
                    .map(|field| {
                        attachment
                            .get(field)
                            .and_then(|value| {
                                value
                                    .as_str()
                                    .map(str::to_string)
                                    .or_else(|| value.as_u64().map(|value| value.to_string()))
                            })
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>();
                for value in &owner {
                    let valid = value.is_empty()
                        || value.chars().all(|ch| ch.is_ascii_digit())
                        || (value
                            .chars()
                            .next()
                            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
                            && value
                                .chars()
                                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')));
                    if !valid {
                        return Err(HyperVError::Config(format!(
                            "invalid {kind} uid/gid for {source}: {value}"
                        )));
                    }
                }
                let chown = if owner.iter().all(|value| value.is_empty()) {
                    String::new()
                } else {
                    format!(
                        " && sudo chown {} {}",
                        shell_quote(&format!("{}:{}", owner[0], owner[1])),
                        shell_quote(&remote_final)
                    )
                };
                self.ssh(
                    &format!(
                        "sudo install -d -m 0755 {dir} && sudo install -m {mode} {tmp} {dest}{chown} && rm -f {tmp}",
                        dir = shell_quote(&remote_dir),
                        mode = shell_quote(&mode),
                        tmp = shell_quote(&remote_tmp),
                        dest = shell_quote(&remote_final),
                        chown = chown,
                    ),
                    true,
                )?;
                args.extend(["-v".to_string(), format!("{remote_final}:{target}:ro")]);
            }
        }
        Ok(())
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
            validate_windows_host(&self.config.windows_host)?;
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            local.to_string_lossy().hash(&mut hasher);
            let share_name = format!("stackdeck_{:x}", hasher.finish());
            let windows_path = local.to_string_lossy();
            let smb_user = &self.config.smb_user;
            let windows_host = &self.config.windows_host;

            let ps_script = format!(
                "$name = '{}'; $path = '{}'; if (-not (Get-NetFirewallRule -DisplayGroup 'File and Printer Sharing' -Enabled True -ErrorAction SilentlyContinue)) {{ Enable-NetFirewallRule -DisplayGroup 'File and Printer Sharing' -ErrorAction SilentlyContinue | Out-Null }}; if (-not (Get-SmbShare -Name $name -ErrorAction SilentlyContinue)) {{ New-SmbShare -Name $name -Path $path -ChangeAccess '{}' | Out-Null }}",
                ps_quote(&share_name), ps_quote(&windows_path), ps_quote(smb_user)
            );
            self.ps(&ps_script, true, None)?;

            let remote = format!("/mnt/stackdeck-shares/{}", share_name);
            let cred = format!("/etc/stackdeck/cifs-{}.cred", share_name);
            let unc = format!("//{}/{}", windows_host, share_name);
            self.install_smb_credentials(&cred)?;
            let mount_cmd = format!(
                "sudo mkdir -p {remote} && if ! mountpoint -q {remote}; then sudo mount -t cifs {unc} {remote} -o credentials={cred},vers=3.0,iocharset=utf8,noserverino || exit 1; fi",
                remote=shell_quote(&remote),
                cred=shell_quote(&cred),
                unc=shell_quote(&unc)
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
        self.remove_portproxy(svc)?;
        if !self.ssh_probe(1) {
            println!(
                "{}: VM is offline; host portproxy entries were removed",
                name
            );
            return Ok(format!(
                "{}: VM is offline; host portproxy entries were removed",
                name
            ));
        }

        let cname = container_name(&svc.project, &svc.name);
        self.ssh(&nerdctl_command(&self.config, &["rm", "-f", &cname]), true)
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

                    if let Some(health_status) = container_health_from_state(&state) {
                        status = health_status.to_string();
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
        if let Some(status) = container_health_from_state(&state) {
            return Ok(status.to_string());
        }
        let running = state
            .get("Running")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(if running { "running" } else { "stopped" }.to_string())
    }

    pub fn service_healthcheck_ok(&self, svc: &HyperVService) -> Result<bool, HyperVError> {
        let Some(hc) = &svc.healthcheck else {
            return Ok(true);
        };
        if hc.r#type == "none" {
            return Ok(true);
        }
        let Some(command) = hc.test.as_ref().and_then(compose_health_command) else {
            return Ok(true);
        };
        let cname = container_name(&svc.project, &svc.name);
        self.exec_health_once(&cname, &command)
    }

    fn supports_nerdctl_health_flags(&self) -> bool {
        let cmd = format!(
            "{} 2>&1 | grep -q -- --health-cmd && echo yes || echo no",
            nerdctl_command(&self.config, &["run", "--help"])
        );
        self.ssh(&cmd, false)
            .map(|out| out.lines().any(|line| line.trim() == "yes"))
            .unwrap_or(false)
    }

    fn wait_for_native_health(
        &self,
        cname: &str,
        hc: &stackdeck_types::HealthCheck,
    ) -> Result<(), HyperVError> {
        let max_duration = health_wait_duration(hc);
        let interval = std::time::Duration::from_secs(hc.interval_seconds.max(1) as u64);
        let start_time = std::time::Instant::now();

        while start_time.elapsed() < max_duration {
            match self.container_health_status(cname)?.as_str() {
                "healthy" => return Ok(()),
                "unhealthy" => break,
                _ => {}
            }
            std::thread::sleep(interval);
        }
        Err(HyperVError::Ssh(format!(
            "{cname}: healthcheck failed or timed out"
        )))
    }

    fn wait_for_exec_health(
        &self,
        cname: &str,
        hc: &stackdeck_types::HealthCheck,
        command: &str,
    ) -> Result<(), HyperVError> {
        let max_duration = health_wait_duration(hc);
        let interval = std::time::Duration::from_secs(hc.interval_seconds.max(1) as u64);
        let start_time = std::time::Instant::now();

        while start_time.elapsed() < max_duration {
            if self.exec_health_once(cname, command)? {
                return Ok(());
            }
            std::thread::sleep(interval);
        }
        Err(HyperVError::Ssh(format!(
            "{cname}: healthcheck failed or timed out"
        )))
    }

    fn exec_health_once(&self, cname: &str, command: &str) -> Result<bool, HyperVError> {
        let exec = nerdctl_command(&self.config, &["exec", cname, "sh", "-lc", command]);
        let probe = format!(
            "if {exec} >/dev/null 2>&1; then echo STACKDECK_HEALTH_OK; else echo STACKDECK_HEALTH_FAIL; fi"
        );
        Ok(self
            .ssh(&probe, false)?
            .lines()
            .any(|line| line.trim() == "STACKDECK_HEALTH_OK"))
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
        self.ssh(&nerdctl_command(&self.config, &args), true)
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
        self.ssh(&nerdctl_command(&self.config, &args), true)
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
        self.ssh(&nerdctl_command(&self.config, &args), true)
    }

    // -----------------------------------------------------------------------
    // Port proxy
    // -----------------------------------------------------------------------

    /// Set up Windows port proxy for service ports.
    pub fn ensure_portproxy(
        &self,
        svc: &stackdeck_types::HyperVService,
    ) -> Result<(), HyperVError> {
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
            let mapping = parse_port_mapping(port)?;
            validate_ssh_host(&self.config.ssh_host)?;
            let host_ip = mapping.host_ip.to_string();
            let host_port = mapping.host_port.to_string();
            let connect_port = mapping.host_port.to_string();
            let _ = self.run_cmd(
                &[
                    "netsh.exe".to_string(),
                    "interface".to_string(),
                    "portproxy".to_string(),
                    "delete".to_string(),
                    "v4tov4".to_string(),
                    format!("listenaddress={host_ip}"),
                    format!("listenport={host_port}"),
                ],
                false,
                Some(30),
            );
            self.run_cmd(
                &[
                    "netsh.exe".to_string(),
                    "interface".to_string(),
                    "portproxy".to_string(),
                    "add".to_string(),
                    "v4tov4".to_string(),
                    format!("listenaddress={host_ip}"),
                    format!("listenport={host_port}"),
                    format!("connectaddress={}", self.config.ssh_host),
                    format!("connectport={connect_port}"),
                ],
                true,
                Some(30),
            )?;
            if mapping.host_ip == Ipv4Addr::UNSPECIFIED {
                let script = format!(
                    "if (-not (Get-NetFirewallRule -DisplayName 'StackDeck Port {host_port}' -ErrorAction SilentlyContinue)) {{ New-NetFirewallRule -DisplayName 'StackDeck Port {host_port}' -Direction Inbound -LocalPort {host_port} -Protocol TCP -Action Allow | Out-Null }}"
                );
                self.ps(&script, true, Some(30))?;
            }
        }
        Ok(())
    }

    /// Remove Windows port proxy for service ports.
    pub fn remove_portproxy(
        &self,
        svc: &stackdeck_types::HyperVService,
    ) -> Result<(), HyperVError> {
        for port in &svc.ports {
            let mapping = parse_port_mapping(port)?;
            let host_ip = mapping.host_ip.to_string();
            let host_port = mapping.host_port.to_string();
            let _ = self.run_cmd(
                &[
                    "netsh.exe".to_string(),
                    "interface".to_string(),
                    "portproxy".to_string(),
                    "delete".to_string(),
                    "v4tov4".to_string(),
                    format!("listenaddress={host_ip}"),
                    format!("listenport={host_port}"),
                ],
                false,
                Some(30),
            );
            if mapping.host_ip == Ipv4Addr::UNSPECIFIED {
                let script = format!(
                    "Remove-NetFirewallRule -DisplayName 'StackDeck Port {host_port}' -ErrorAction SilentlyContinue | Out-Null"
                );
                self.ps(&script, false, Some(30)).ok();
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Health check
    // -----------------------------------------------------------------------

    /// Fail closed unless the Hyper-V guest and the container runtime are
    /// actually usable.  This is the release/first-run success gate; callers
    /// must not infer readiness from cached images or a partially created VM.
    pub fn assert_runtime_ready(&self) -> Result<(), HyperVError> {
        let state = self
            .ps(
                &format!(
                    "(Get-VM -Name '{}' -ErrorAction SilentlyContinue).State",
                    self.config.vm_name.replace('\'', "''")
                ),
                false,
                Some(10),
            )?
            .trim()
            .to_string();
        if !state.eq_ignore_ascii_case("running") {
            return Err(HyperVError::Config(format!(
                "Hyper-V VM '{}' is not running (state: {}).",
                self.config.vm_name,
                if state.is_empty() { "unknown" } else { &state }
            )));
        }
        if !self.ssh_probe(5) {
            return Err(HyperVError::Ssh(format!(
                "Hyper-V VM '{}' is running but SSH is not reachable.",
                self.config.vm_name
            )));
        }
        let nerdctl = nerdctl_command(&self.config, &["ps", "-a"]);
        let command = format!(
            "systemctl is-active --quiet containerd && command -v nerdctl >/dev/null 2>&1 && {nerdctl} >/dev/null && systemctl is-active --quiet buildkit && buildctl --addr unix:///run/buildkit/buildkitd.sock debug workers >/dev/null 2>&1"
        );
        let mut args = self.ssh_base();
        args.push(command);
        self.run_cmd(&args, true, Some(20))?;
        Ok(())
    }

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

            let buildkit = self
                .ssh(
                    "systemctl is-active buildkit 2>/dev/null && buildctl --addr unix:///run/buildkit/buildkitd.sock debug workers >/dev/null 2>&1 && echo ready || echo unavailable",
                    false,
                )
                .unwrap_or_default();
            health.insert("buildkit".into(), serde_json::json!(buildkit));
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

fn parse_hyperv_config(text: &str) -> Result<HyperVConfig, serde_json::Error> {
    // Windows PowerShell 5.1 commonly writes UTF-8 JSON with a BOM. serde_json
    // intentionally rejects that marker, so normalize only the optional BOM
    // before parsing while preserving all other strict JSON validation.
    serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(text))
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

fn push_ssh_options(args: &mut Vec<String>) {
    for option in NONINTERACTIVE_SSH_OPTIONS {
        args.extend(["-o".to_string(), option.to_string()]);
    }
}

fn pump_output<R>(
    mut reader: R,
    stream: CommandOutputStream,
    sender: std::sync::mpsc::SyncSender<(CommandOutputStream, Vec<u8>)>,
) where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(count) if sender.send((stream, buffer[..count].to_vec())).is_err() => return,
                Ok(_) => {}
            }
        }
    });
}

fn output_with_timeout(
    command: &mut Command,
    timeout_secs: Option<u64>,
) -> std::io::Result<(Output, bool)> {
    let Some(timeout_secs) = timeout_secs else {
        return command.output().map(|output| (output, false));
    };

    wait_child_output_with_timeout(command.spawn()?, Some(timeout_secs))
}

fn wait_child_output_with_timeout(
    mut child: Child,
    timeout_secs: Option<u64>,
) -> std::io::Result<(Output, bool)> {
    let stdout_reader = child.stdout.take().map(drain_captured_output);
    let stderr_reader = child.stderr.take().map(drain_captured_output);
    let timeout = timeout_secs.map(|seconds| Duration::from_secs(seconds.max(1)));
    let started = Instant::now();
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (status, false);
        }

        if timeout.is_some_and(|timeout| started.elapsed() >= timeout) {
            let _ = child.kill();
            break (child.wait()?, true);
        }

        let sleep_for = timeout
            .and_then(|timeout| timeout.checked_sub(started.elapsed()))
            .unwrap_or_else(|| Duration::from_millis(50));
        thread::sleep(sleep_for.min(Duration::from_millis(50)));
    };
    let stdout = join_captured_output(stdout_reader)?;
    let stderr = join_captured_output(stderr_reader)?;
    Ok((
        Output {
            status,
            stdout,
            stderr,
        },
        timed_out,
    ))
}

fn drain_captured_output<R>(mut reader: R) -> thread::JoinHandle<std::io::Result<Vec<u8>>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut captured = Vec::new();
        let mut chunk = [0_u8; 8192];
        loop {
            let count = reader.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            if count >= MAX_CAPTURED_OUTPUT_BYTES {
                captured.clear();
                captured.extend_from_slice(&chunk[count - MAX_CAPTURED_OUTPUT_BYTES..count]);
            } else {
                let excess = captured
                    .len()
                    .saturating_add(count)
                    .saturating_sub(MAX_CAPTURED_OUTPUT_BYTES);
                if excess > 0 {
                    captured.drain(..excess);
                }
                captured.extend_from_slice(&chunk[..count]);
            }
        }
        Ok(captured)
    })
}

fn join_captured_output(
    reader: Option<thread::JoinHandle<std::io::Result<Vec<u8>>>>,
) -> std::io::Result<Vec<u8>> {
    match reader {
        Some(reader) => reader
            .join()
            .map_err(|_| std::io::Error::other("command output reader thread panicked"))?,
        None => Ok(Vec::new()),
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
    let raw = format!("stackdeck-{}-{}", project, service).to_lowercase();
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
        format!("stackdeck-{}", slug)
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

fn nerdctl_health_args(hc: &stackdeck_types::HealthCheck) -> Vec<String> {
    if hc.r#type == "none" {
        return Vec::new();
    }
    let Some(command) = hc.test.as_ref().and_then(compose_health_command) else {
        return Vec::new();
    };

    let mut args = vec!["--health-cmd".to_string(), command];
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
    args
}

fn health_wait_duration(hc: &stackdeck_types::HealthCheck) -> std::time::Duration {
    std::time::Duration::from_secs(
        (hc.start_period_seconds + (hc.interval_seconds + hc.timeout_seconds) * hc.retries + 10)
            as u64,
    )
}

fn container_health_from_state(state: &serde_json::Value) -> Option<&str> {
    state.pointer("/Health/Status").and_then(|v| v.as_str())
}

fn container_is_running(inspect_json: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(inspect_json) else {
        return false;
    };
    let item = parsed
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or(&parsed);
    item.pointer("/State/Running")
        .and_then(|value| value.as_bool())
        .unwrap_or_else(|| {
            item.pointer("/State/Status")
                .and_then(|value| value.as_str())
                .is_some_and(|status| status.eq_ignore_ascii_case("running"))
        })
}

fn service_config_hash(svc: &HyperVService) -> Result<String, HyperVError> {
    let serialized = serde_json::to_vec(svc)?;
    let digest = Sha256::digest(serialized);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn container_config_hash(inspect_json: &str) -> Option<String> {
    let parsed = serde_json::from_str::<serde_json::Value>(inspect_json).ok()?;
    let item = parsed
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or(&parsed);
    item.pointer("/Config/Labels/stackdeck.config-hash")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn materialize_resource_file(
    root: &Path,
    resource: &serde_json::Value,
    kind: &str,
    source: &str,
) -> Result<(PathBuf, bool), HyperVError> {
    if let Some(file) = resource.get("file").and_then(|value| value.as_str()) {
        let root = root.canonicalize().map_err(|error| {
            HyperVError::Config(format!(
                "failed to canonicalize project root {}: {error}",
                root.display()
            ))
        })?;
        let path = PathBuf::from(file);
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        let path = path.canonicalize().map_err(|error| {
            HyperVError::Config(format!(
                "failed to resolve {kind} file {}: {error}",
                path.display()
            ))
        })?;
        if !path.starts_with(&root) || !path.is_file() {
            return Err(HyperVError::Config(format!(
                "{kind} file must be a regular file inside the project root: {}",
                path.display()
            )));
        }
        return Ok((path, false));
    }

    let contents = if let Some(contents) = resource.get("content").and_then(|value| value.as_str())
    {
        contents.to_string()
    } else if let Some(variable) = resource.get("environment").and_then(|value| value.as_str()) {
        std::env::var(variable).map_err(|_| {
            HyperVError::Config(format!(
                "environment variable '{variable}' for {kind} '{source}' is not set"
            ))
        })?
    } else {
        return Err(HyperVError::Config(format!(
            "{kind} '{source}' must define file, content, or environment"
        )));
    };
    let path = std::env::temp_dir().join(format!(
        "stackdeck-resource-{}-{}-{}",
        std::process::id(),
        project_slug(source),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    stackdeck_types::atomic_write(&path, contents.as_bytes())?;
    Ok((path, true))
}

fn is_udp_port(port: &str) -> bool {
    port.trim().to_ascii_lowercase().ends_with("/udp")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PortMapping {
    host_ip: Ipv4Addr,
    host_port: u16,
    container_port: u16,
    protocol: String,
}

fn parse_port_mapping(port: &str) -> Result<PortMapping, HyperVError> {
    let text = port.trim().trim_matches('"').trim_matches('\'');
    if text.is_empty() {
        return Err(HyperVError::Config(
            "published port must not be empty".into(),
        ));
    }
    let (text, protocol) = text
        .rsplit_once('/')
        .map(|(base, protocol)| (base, protocol.to_ascii_lowercase()))
        .unwrap_or((text, "tcp".to_string()));
    if protocol != "tcp" && protocol != "udp" {
        return Err(HyperVError::Config(format!(
            "unsupported published port protocol: {protocol}"
        )));
    }
    let parts: Vec<&str> = text.split(':').collect();
    let (host_ip, host_port, container_port) = match parts.as_slice() {
        [single] => {
            let port = single
                .parse::<u16>()
                .map_err(|_| HyperVError::Config(format!("invalid published port: {text}")))?;
            (Ipv4Addr::LOCALHOST, port, port)
        }
        [host, container] => (
            Ipv4Addr::LOCALHOST,
            host.parse::<u16>()
                .map_err(|_| HyperVError::Config(format!("invalid host port: {host}")))?,
            container
                .parse::<u16>()
                .map_err(|_| HyperVError::Config(format!("invalid container port: {container}")))?,
        ),
        [host_ip, host, container] => {
            let host_ip = if host_ip.is_empty() {
                Ipv4Addr::LOCALHOST
            } else {
                host_ip.parse::<Ipv4Addr>().map_err(|_| {
                    HyperVError::Config(format!(
                        "invalid IPv4 listen address in published port: {host_ip}"
                    ))
                })?
            };
            (
                host_ip,
                host.parse::<u16>()
                    .map_err(|_| HyperVError::Config(format!("invalid host port: {host}")))?,
                container.parse::<u16>().map_err(|_| {
                    HyperVError::Config(format!("invalid container port: {container}"))
                })?,
            )
        }
        _ => {
            return Err(HyperVError::Config(
                "IPv6 published listen addresses are not supported by Windows v4tov4 portproxy"
                    .into(),
            ))
        }
    };
    if host_port == 0 || container_port == 0 {
        return Err(HyperVError::Config(
            "published host and container ports must be between 1 and 65535".into(),
        ));
    }
    Ok(PortMapping {
        host_ip,
        host_port,
        container_port,
        protocol,
    })
}

/// Parse a published port into (host_ip, host_port, container_port).
pub fn parse_port(port: &str) -> Option<(String, u16, u16)> {
    let mapping = parse_port_mapping(port).ok()?;
    Some((
        mapping.host_ip.to_string(),
        mapping.host_port,
        mapping.container_port,
    ))
}

fn nerdctl_publish_arg(port: &str) -> Option<String> {
    let mapping = parse_port_mapping(port).ok()?;
    let suffix = if mapping.protocol == "tcp" {
        String::new()
    } else {
        format!("/{}", mapping.protocol)
    };
    // HostIp is a Windows host policy enforced by portproxy. The guest must
    // listen on its VM-reachable interface for the proxy to connect.
    Some(format!(
        "{}:{}{}",
        mapping.host_port, mapping.container_port, suffix
    ))
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
    if !cfg.windows_host.trim().is_empty() {
        validate_windows_host(&cfg.windows_host)?;
    }
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

fn validate_windows_host(host: &str) -> Result<(), HyperVError> {
    let host = host.trim();
    if host.parse::<Ipv4Addr>().is_ok() {
        return Ok(());
    }
    let valid = !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        });
    if valid {
        Ok(())
    } else {
        Err(HyperVError::Config(format!(
            "invalid windows_host '{}': expected an IPv4 address or DNS hostname",
            host
        )))
    }
}

fn validate_registry_mirror(registry: &str, endpoints: &[String]) -> Result<(), HyperVError> {
    validate_registry_name(registry)?;
    if endpoints.is_empty() {
        return Err(HyperVError::Config(
            "at least one registry mirror endpoint is required".into(),
        ));
    }
    for endpoint in endpoints {
        let endpoint = endpoint.trim();
        let valid = (endpoint.starts_with("https://") || endpoint.starts_with("http://"))
            && !endpoint.chars().any(|character| {
                character.is_ascii_control()
                    || character.is_ascii_whitespace()
                    || matches!(character, '"' | '\'' | '`' | '$' | '\\')
            });
        if !valid {
            return Err(HyperVError::Config(format!(
                "invalid registry mirror endpoint: {endpoint}"
            )));
        }
    }
    Ok(())
}

fn validate_registry_name(registry: &str) -> Result<(), HyperVError> {
    let registry = registry.trim();
    let valid_registry = !registry.is_empty()
        && registry.len() <= 253
        && !registry.contains("..")
        && registry.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | ':' | '[' | ']')
        });
    if !valid_registry {
        return Err(HyperVError::Config(
            "mirror registry must be a valid host[:port]".into(),
        ));
    }
    Ok(())
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

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|v| v.to_str())
        .map(|v| v.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

fn is_tar_archive(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.ends_with(".tar")
        || name.ends_with(".tar.gz")
        || name.ends_with(".tgz")
        || name.ends_with(".ova")
}

fn archive_stem(path: &Path) -> String {
    let mut name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("cloud-image")
        .to_string();
    for suffix in [".tar.gz", ".tgz", ".tar", ".ova"] {
        if name.to_ascii_lowercase().ends_with(suffix) {
            let keep = name.len() - suffix.len();
            name.truncate(keep);
            break;
        }
    }
    project_slug(&name)
}

fn find_disk_image(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut convertible = None;
    let mut vhd = None;
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if has_extension(&path, "vhdx") {
                return Some(path);
            } else if has_extension(&path, "vhd") && vhd.is_none() {
                vhd = Some(path);
            } else if (has_extension(&path, "vmdk")
                || has_extension(&path, "qcow2")
                || has_extension(&path, "img"))
                && convertible.is_none()
            {
                convertible = Some(path);
            }
        }
    }
    vhd.or(convertible)
}

fn ps_quote(value: &str) -> String {
    value.replace('\'', "''")
}

fn cloud_init_user_data(user: &str, public_key: &str) -> String {
    format!(
        r#"#cloud-config
users:
  - name: {user}
    groups: [adm, sudo]
    shell: /bin/bash
    sudo: ['ALL=(ALL) NOPASSWD:ALL']
    lock_passwd: true
    ssh_authorized_keys:
      - {key}
ssh_pwauth: false
disable_root: true
package_update: true
packages:
  - openssh-server
  - ca-certificates
  - curl
  - gnupg
  - containerd
  - cifs-utils
  - cloud-guest-utils
  - containernetworking-plugins
  - uidmap
  - iptables
runcmd:
  - [ sh, -c, "systemctl enable --now ssh || systemctl enable --now sshd" ]
  - [ systemctl, enable, --now, containerd ]
  - [ growpart, /dev/sda, '1' ]
  - [ resize2fs, /dev/sda1 ]
final_message: "stackdeck cloud-init complete"
"#,
        user = user,
        key = public_key.replace('\n', " "),
    )
}

fn azure_ovf_env(hostname: &str, user: &str, public_key: &str) -> String {
    let user_data = cloud_init_user_data(user, public_key);
    let custom_data = base64_encode(user_data.as_bytes());
    let hostname = xml_escape(hostname);
    let user = xml_escape(user);
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<wa:Environment xmlns:wa="http://schemas.dmtf.org/ovf/environment/1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <wa:ProvisioningSection>
    <wa:Version>1.0</wa:Version>
    <LinuxProvisioningConfigurationSet xmlns="http://schemas.microsoft.com/windowsazure" xmlns:i="http://www.w3.org/2001/XMLSchema-instance">
      <ConfigurationSetType>LinuxProvisioningConfiguration</ConfigurationSetType>
      <HostName>{hostname}</HostName>
      <UserName>{user}</UserName>
      <UserPassword/>
      <CustomData>{custom_data}</CustomData>
      <DisableSshPasswordAuthentication>true</DisableSshPasswordAuthentication>
      <SSH>
        <PublicKeys>
          <PublicKey>
            <Fingerprint>0000000000000000000000000000000000000000</Fingerprint>
            <Path>/home/{user}/.ssh/authorized_keys</Path>
          </PublicKey>
        </PublicKeys>
      </SSH>
    </LinuxProvisioningConfigurationSet>
  </wa:ProvisioningSection>
</wa:Environment>
"#
    )
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// PowerShell that builds the cloud-init NoCloud seed ISO. Prefers `oscdimg`,
/// then `genisoimage`; otherwise falls back to the built-in Windows IMAPI2 COM
/// (`IMAPI2FS.MsftFileSystemImage`) so no external tool is required. Produces a
/// `cidata`-labeled ISO9660+Joliet+UDF image with `user-data`/`meta-data` and
/// Azure-compatible `ovf-env.xml` at root.
/// Placeholders `@@SEED@@` and `@@ISO@@` are substituted by the caller.
const ISO_BUILD_SEED_PS: &str = r##"
$ErrorActionPreference = 'Stop'
$seed = '@@SEED@@'
$iso = '@@ISO@@'
if (Test-Path -LiteralPath $iso -PathType Leaf) { throw "Seed ISO already exists; refusing to overwrite it." }
$oscdimg = Get-Command oscdimg.exe -ErrorAction SilentlyContinue
$geniso = Get-Command genisoimage.exe -ErrorAction SilentlyContinue
if ($oscdimg) {
  & $oscdimg.Source -o -m -j2 -u2 -udfver102 -lCIDATA $seed $iso | Out-Null
} elseif ($geniso) {
  & $geniso.Source -output $iso -volid cidata -joliet -rock -udf $seed
} else {
  if (-not ('StackDeckIso' -as [type])) {
    Add-Type -Language CSharp -TypeDefinition '
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
public static class StackDeckIso {
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
  $fsi.FileSystemsToCreate = 7
  $fsi.VolumeName = 'cidata'
  [void]$fsi.Root.AddTree($seed, $false)
  [StackDeckIso]::Write($fsi.CreateResultImage().ImageStream, $iso)
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
    fn vm_creation_requires_healthy_inventory_and_refuses_existing_disks() {
        let guard = VM_CREATION_SAFETY_GUARD;
        assert!(guard.contains("Get-CimInstance"));
        assert!(guard.contains("Msvm_ComputerSystem"));
        assert!(guard.contains("Get-VM -ErrorAction Stop"));
        assert!(guard.contains("Test-Path -LiteralPath $disk"));
        assert!(guard.contains("throw"));
        assert!(!guard.contains("Remove-VM"));
        assert!(!guard.contains("SilentlyContinue"));

        let source = include_str!("lib.rs");
        let placeholder = ["{", "vm_safety_guard", "}"].concat();
        assert_eq!(source.matches(&placeholder).count(), 2);
        let preflight = ["self.ensure_vm_creation_is_safe()", "?;"].concat();
        assert_eq!(source.matches(&preflight).count(), 3);
        assert!(guard.contains("seed.iso"));
        assert!(guard.contains("seed.vhdx"));
        let final_guard = VM_FINAL_CREATION_SAFETY_GUARD;
        assert!(final_guard.contains("Get-CimInstance"));
        assert!(final_guard.contains("Get-VM -ErrorAction Stop"));
        assert!(final_guard.contains("Test-Path -LiteralPath $disk"));
        assert!(!final_guard.contains("seed.iso"));
        assert!(!final_guard.contains("seed.vhdx"));
        assert!(!final_guard.contains("Remove-VM"));
        let dangerous_converted_delete = ["Remove-Item -Force", " $out"].concat();
        let dangerous_seed_delete = ["Remove-Item -Force", " $vhd"].concat();
        let dangerous_vm_copy = ["Copy-Item -Force", " $src $disk"].concat();
        assert!(!source.contains(&dangerous_converted_delete));
        assert!(!source.contains(&dangerous_seed_delete));
        assert!(!source.contains(&dangerous_vm_copy));
    }

    #[test]
    fn test_container_name() {
        assert_eq!(container_name("myapp", "web"), "stackdeck-myapp-web");
        assert_eq!(
            container_name("My-App", "Web Server"),
            "stackdeck-my-app-web-server"
        );
    }

    #[test]
    fn test_project_network_name() {
        assert_eq!(project_network_name("myapp"), "stackdeck-myapp");
        assert_eq!(
            project_network_name("Smart Surveillance SaaS MVP"),
            "stackdeck-smart-surveillance-saas-mvp"
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
        assert!(parse_port_mapping("127.0.0.1;Start-Process calc:8080:80").is_err());
        assert!(parse_port_mapping("127.0.0.1:8080:80;whoami").is_err());
        assert_eq!(
            parse_port_mapping("0.0.0.0:5353:53/udp").unwrap().protocol,
            "udp"
        );
    }

    #[test]
    fn test_nerdctl_publish_arg_uses_vm_reachable_binding() {
        assert_eq!(
            nerdctl_publish_arg("127.0.0.1:8080:80").as_deref(),
            Some("8080:80")
        );
        assert_eq!(
            nerdctl_publish_arg("0.0.0.0:8080:80").as_deref(),
            Some("8080:80")
        );
        assert_eq!(nerdctl_publish_arg("8080:80").as_deref(), Some("8080:80"));
        assert_eq!(
            nerdctl_publish_arg("0.0.0.0:5353:53/udp").as_deref(),
            Some("5353:53/udp")
        );
    }

    #[test]
    fn test_nerdctl_health_args_use_container_native_healthcheck() {
        let hc = stackdeck_types::HealthCheck {
            r#type: "command".into(),
            test: Some(serde_json::json!([
                "CMD-SHELL",
                "curl -fsS http://localhost || exit 1"
            ])),
            interval_seconds: 5,
            timeout_seconds: 3,
            retries: 4,
            start_period_seconds: 10,
            ..Default::default()
        };

        let args = nerdctl_health_args(&hc);

        assert_eq!(args[0], "--health-cmd");
        assert_eq!(args[1], "curl -fsS http://localhost || exit 1");
        assert!(args.windows(2).any(|w| w == ["--health-interval", "5s"]));
        assert!(args.windows(2).any(|w| w == ["--health-timeout", "3s"]));
        assert!(args.windows(2).any(|w| w == ["--health-retries", "4"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["--health-start-period", "10s"]));
    }

    #[test]
    fn test_container_health_from_state_reports_healthy_status() {
        let state = serde_json::json!({
            "Status": "running",
            "Running": true,
            "Health": {
                "Status": "healthy"
            }
        });

        assert_eq!(container_health_from_state(&state), Some("healthy"));
    }

    #[test]
    fn test_project_slug() {
        assert_eq!(project_slug("My Project"), "my-project");
        assert_eq!(project_slug("test"), "test");
        assert_eq!(project_slug("---"), "");
    }

    #[test]
    fn test_load_save_config() {
        let dir = std::env::temp_dir().join("stackdeck_hyperv_test");
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
    fn test_parse_hyperv_config_accepts_utf8_bom_and_applies_defaults() {
        let parsed = parse_hyperv_config(
            "\u{feff}{\"vm_root\":\"D:\\\\StackDeck\\\\VMs\\\\stackdeck-linux\"}",
        )
        .unwrap();

        assert_eq!(parsed.vm_root, r"D:\StackDeck\VMs\stackdeck-linux");
        assert_eq!(parsed.vm_name, "stackdeck-linux");
        assert_eq!(parsed.vm_disk_gb, 10);
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
        assert!(cmd.contains("stackdeck"));
        assert!(cmd.contains("ps"));
    }

    #[test]
    fn test_nerdctl_command_quotes_namespace() {
        let cfg = HyperVConfig {
            namespace: "stackdeck; touch /tmp/pwn".into(),
            ..HyperVConfig::default()
        };
        let cmd = nerdctl_command(&cfg, &["ps"]);
        assert!(cmd.contains("--namespace 'stackdeck; touch /tmp/pwn'"));
    }

    #[test]
    fn test_validate_config_rejects_unsafe_shell_fields() {
        let cfg = HyperVConfig {
            namespace: "stackdeck;touch".into(),
            ssh_host: "127.0.0.1".into(),
            ..HyperVConfig::default()
        };
        assert!(validate_config_for_shell(&cfg).is_err());

        let cfg = HyperVConfig {
            namespace: "stackdeck".into(),
            ssh_host: "127.0.0.1; calc".into(),
            ..HyperVConfig::default()
        };
        assert!(validate_config_for_shell(&cfg).is_err());

        let cfg = HyperVConfig {
            namespace: "stackdeck".into(),
            ssh_host: "vm-host.local".into(),
            ..HyperVConfig::default()
        };
        assert!(validate_config_for_shell(&cfg).is_ok());

        let cfg = HyperVConfig {
            namespace: "stackdeck".into(),
            ssh_host: "127.0.0.1".into(),
            windows_host: "host; curl attacker".into(),
            ..HyperVConfig::default()
        };
        assert!(validate_config_for_shell(&cfg).is_err());
    }

    #[test]
    fn test_registry_mirror_validation_rejects_path_and_shell_injection() {
        assert!(validate_registry_mirror(
            "registry.example:5000",
            &["https://mirror.example".to_string()]
        )
        .is_ok());
        assert!(
            validate_registry_mirror("../../etc", &["https://mirror.example".to_string()]).is_err()
        );
        assert!(validate_registry_mirror(
            "registry.example",
            &["https://mirror.example/$(id)".to_string()]
        )
        .is_err());
    }

    #[test]
    fn test_container_running_state_is_strict() {
        assert!(container_is_running(r#"[{"State":{"Running":true}}]"#));
        assert!(!container_is_running(
            r#"[{"State":{"Running":false,"Status":"exited"}}]"#
        ));
        assert!(!container_is_running("not json"));
    }

    #[test]
    fn service_configuration_hash_is_recorded_and_detected() {
        let service = HyperVService {
            project: "project".into(),
            name: "web".into(),
            image: "alpine:latest".into(),
            ..HyperVService::default()
        };
        let hash = service_config_hash(&service).unwrap();
        assert_eq!(hash.len(), 64);
        let inspect = serde_json::json!([{
            "Config": { "Labels": { "stackdeck.config-hash": hash } }
        }]);
        assert_eq!(
            container_config_hash(&inspect.to_string()).as_deref(),
            Some(hash.as_str())
        );
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
    fn test_cloud_init_installs_and_starts_ssh() {
        let user_data = cloud_init_user_data("stackdeck", "ssh-ed25519 AAAATEST user@example");

        assert!(user_data.contains("  - openssh-server"));
        assert!(user_data.contains("systemctl enable --now ssh"));
        assert!(user_data.contains("ssh_authorized_keys:"));
        assert!(user_data.contains("ssh-ed25519 AAAATEST user@example"));
    }

    #[test]
    fn test_azure_ovf_contains_custom_data() {
        let ovf = azure_ovf_env(
            "stackdeck-vm",
            "stackdeck",
            "ssh-ed25519 AAAATEST user@example",
        );

        assert!(ovf.contains("<LinuxProvisioningConfigurationSet"));
        assert!(ovf.contains("<HostName>stackdeck-vm</HostName>"));
        assert!(ovf.contains("<UserName>stackdeck</UserName>"));
        assert!(ovf.contains("<CustomData>"));
        assert!(ovf.contains(&base64_encode(
            cloud_init_user_data("stackdeck", "ssh-ed25519 AAAATEST user@example").as_bytes()
        )));
    }

    #[test]
    fn test_archive_helpers_recognize_ubuntu_vhd_tarball() {
        let path = Path::new("jammy-server-cloudimg-amd64-azure.vhd.tar.gz");

        assert!(is_tar_archive(path));
        assert_eq!(archive_stem(path), "jammy-server-cloudimg-amd64-azure.vhd");

        let ova = Path::new("jammy-server-cloudimg-amd64.ova");
        assert!(is_tar_archive(ova));
        assert_eq!(archive_stem(ova), "jammy-server-cloudimg-amd64");
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

    #[cfg(windows)]
    #[test]
    fn timed_command_drains_large_piped_output_before_exit() {
        let mut command = create_command("powershell");
        command
            .args([
                "-NoProfile",
                "-Command",
                "[Console]::Out.Write(('x' * 1048576))",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let (output, timed_out) = output_with_timeout(&mut command, Some(10)).unwrap();
        assert!(!timed_out);
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 1_048_576);
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
