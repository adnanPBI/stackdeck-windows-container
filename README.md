# StackDeck / PyStack Hyper-V Container Runtime

This repository is a **Rust-first Windows container orchestration project**. It is not a Python package and it does not ship a Python zipapp. The main deliverables are:

- `pystack` / `hive`: Rust CLI binaries
- `pystack-hyperv`: Hyper-V backend crate
- `pystack-api`: local Docker Engine API compatibility shim backed by the Hyper-V VM
- `desktop/stackdeck`: Tauri 2 + React desktop UI

## Architecture contract

The intended runtime architecture is:

```text
Windows 11 host
  -> Rust CLI / Tauri desktop
  -> Hyper-V VM
  -> containerd + nerdctl inside VM
  -> OCI containers
```

The project is designed to be independent from:

```text
WSL / WSL2
Docker Desktop
host docker.exe
host dockerd / Docker Engine
host docker.sock
```

It intentionally depends on Windows/Hyper-V tooling:

```text
Windows 11 Pro/Enterprise/Education with Hyper-V
Administrator PowerShell
Microsoft Hyper-V PowerShell module
OpenSSH client: ssh, scp, ssh-keygen
qemu-img.exe for the default Debian .qcow2 path, or a pre-converted dynamic VHDX cloud image
oscdimg.exe or genisoimage.exe are optional; Windows IMAPI2 is used as a fallback for cloud-init seed ISO creation
SMB/CIFS for live host bind mounts
netsh portproxy + Windows Firewall
containerd + nerdctl inside the Hyper-V VM
```


## Local deployment package

For Windows 10/11 local deployment, start here:

```powershell
Set-ExecutionPolicy -Scope Process Bypass -Force
.\scripts\windows\00-Ensure-Prereqs.ps1
.\scripts\windows\01-Build-Release.ps1 -SkipDesktop
.\scripts\windows\02-Configure-HyperV.ps1 -PystackExe .\target\release\pystack.exe -SmbPassword "<your-windows-password>"
.\scripts\windows\03-Init-HyperV-Runtime.ps1 -PystackExe .\target\release\pystack.exe
.\scripts\windows\04-Smoke-Test.ps1 -PystackExe .\target\release\pystack.exe
```

Detailed phase-by-phase guide: `docs/LOCAL_DEPLOYMENT_WINDOWS.md`.

## Build

```powershell
cargo fmt --check
cargo test --workspace --locked
cargo build --workspace --locked
cargo build -p pystack-cli --bin pystack --release --locked
cargo build -p pystack-cli --bin hive --release --locked
```

Desktop build:

```powershell
cd desktop\stackdeck
npm ci
npm run build
npm run package:windows
```

The desktop package stages the sidecar as both:

```text
desktop/stackdeck/src-tauri/bin/stackdeck-hive-x86_64-pc-windows-msvc.exe
desktop/stackdeck/src-tauri/bin/stackdeck-hive.exe
```

Generated binaries are release artifacts and should not be committed.

## First-time Hyper-V setup

Run PowerShell as Administrator.

```powershell
Enable-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All -All -NoRestart
Enable-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-Management-PowerShell -All -NoRestart
```

Restart Windows if Hyper-V was newly enabled.

Create a VM root on a drive with enough free space with a small 10 GB dynamic VHDX target:

```powershell
New-Item -ItemType Directory -Force -Path "$env:SystemDrive\StackDeck\VMs\pystack-linux" | Out-Null
New-Item -ItemType Directory -Force -Path "$env:SystemDrive\StackDeck\Images" | Out-Null

pystack hyperv configure `
  --vm-name "pystack-linux" `
  --vm-root "$env:SystemDrive\StackDeck\VMs\pystack-linux" `
  --disk-gb 10 `
  --memory-mb 4096 `
  --cpus 2 `
  --switch-name "Default Switch" `
  --portproxy true
```

Generate or confirm the SSH key:

```powershell
pystack hyperv ensure-key
```

Initialize from the default Debian cloud image. Pass `-Sha256` if you want explicit checksum verification:

```powershell
pystack hyperv init `
  --url "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-amd64.qcow2" `
  --timeout 900
```

The init flow downloads the image, verifies SHA-256, converts to dynamic VHDX when needed, creates cloud-init seed ISO, creates the Hyper-V VM, discovers IP, waits for SSH, and bootstraps containerd/nerdctl.

## SMB live bind mounts

Live host bind mounts require SMB config. The Hyper-V backend no longer silently copies bind mounts with SCP. If a service uses a bind mount and SMB is not configured, startup fails with a configuration error.

Configure SMB access:

```powershell
pystack hyperv configure `
  --windows-host "<Windows-hostname-or-IP>" `
  --smb-user "<WindowsUser>" `
  --smb-password "<WindowsPassword>"
```

Create or mount shares:

```powershell
pystack hyperv share add --path "D:\Projects\my-app" --name my-app
pystack hyperv share mount --name my-app
```

## Compose startup and health semantics

Compose dependency conditions are preserved:

```yaml
services:
  web:
    depends_on:
      db:
        condition: service_healthy
```

When starting a target service, required dependencies are auto-included and started first. Hyper-V startup waits for `service_started`, `service_healthy`, or `service_completed_successfully` based on the Compose condition.

Start a stack:

```powershell
pystack compose up -f docker-compose.yml --backend hyperv -d
```

Supervise services:

```powershell
pystack compose up -f docker-compose.yml --backend hyperv --supervise --interval 10
```

For Hyper-V services with healthchecks, supervision treats non-healthy containers as bad and restarts them unless `restart: no`.

## Diagnostics bundle

Generate a redacted diagnostics directory bundle:

```powershell
pystack diagnostics --config stack.json --output .pystack\diagnostics --tail 200
```

The bundle contains:

```text
summary.md
stack.redacted.json
status.redacted.json
logs/*.redacted.log
manifest.json
```

For a single text report:

```powershell
pystack diagnostics --config stack.json --output diagnostics.txt --text
```

## Docker API shim

The shim listens on localhost by default and is backed by the Hyper-V runtime, not host Docker Engine:

```powershell
pystack daemon serve --host 127.0.0.1 --port 23750
```

Authentication is required unless explicitly running in insecure local mode through the configured CLI/API option.

## Repository hygiene

Do not commit generated artifacts:

```text
target/
desktop/stackdeck/src-tauri/bin/*.exe
*.log
*.pdb
*.dll
```

See `.gitignore` for enforced ignore rules.

## Security Warning: SMB Passwords

**StackDeck generates and stores an SMB password in plaintext (stack.json)** to facilitate Hyper-V live bind mounts. This architecture is designed strictly as a **local-only development tool** for individual developers on trusted Windows machines. It does not integrate with Windows DPAPI or Credential Manager. Do not deploy this runtime architecture on shared servers or production environments without hardening the credential storage layer.
