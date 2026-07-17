# Local deployment guide: Windows 10/11 Hyper-V runtime

This guide turns the repository into a local Windows deployment for the production MVP architecture:

```text
CLI / StackDeck UI
  -> local PyStack runtime commands / optional local API task
  -> Hyper-V Linux VM
  -> containerd + nerdctl inside VM
  -> OCI containers
```

## Non-goals for this MVP

- No WSL or WSL2.
- No Docker Desktop.
- No host Docker Engine or host `docker.exe` dependency.
- No UDP port publishing yet. TCP publishing is supported; UDP must fail clearly.
- No DPAPI/Credential Manager yet. SMB credentials are stored as local config with ACL guidance.

## Phase 0: unpack and open Admin PowerShell

Run every install/init step in **Administrator PowerShell**.

```powershell
Set-ExecutionPolicy -Scope Process Bypass -Force
cd <repo-root>
```

## Phase 1: prerequisite check

```powershell
.\scripts\windows\00-Ensure-Prereqs.ps1
```

To enable Hyper-V features:

```powershell
.\scripts\windows\00-Ensure-Prereqs.ps1 -EnableFeatures
```

Restart Windows if Hyper-V was newly enabled.

Required tools:

```text
Hyper-V PowerShell module
OpenSSH ssh/scp/ssh-keygen
tar
Rust cargo/rustc
qemu-img unless using preconverted VHDX
oscdimg or genisoimage for cloud-init seed ISO
node/npm only for desktop package build
```

## Phase 2: build release binaries

```powershell
.\scripts\windows\01-Build-Release.ps1
```

For CLI-only deployment:

```powershell
.\scripts\windows\01-Build-Release.ps1 -SkipDesktop
```

Release binaries:

```text
target\release\pystack.exe
target\release\hive.exe
```

## Phase 3: configure Hyper-V runtime on D:

```powershell
.\scripts\windows\02-Configure-HyperV.ps1 `
  -PystackExe .\target\release\pystack.exe `
  -VmRoot "D:\StackDeck\VMs\pystack-linux" `
  -DiskGB 10 `
  -MemoryMB 4096 `
  -Cpus 2 `
  -SwitchName "Default Switch" `
  -WindowsHost $env:COMPUTERNAME `
  -SmbUser "$env:USERDOMAIN\$env:USERNAME" `
  -SmbPassword "<your-windows-password>"
```

The VHDX is configured as a small 10 GB virtual disk. Hyper-V dynamic VHDX grows physically up to this virtual size. Expansion beyond 10 GB requires explicit `Resize-VHD` + Linux filesystem grow or a future app-managed disk reconciler.

## Phase 4: initialize the VM

Use an official Ubuntu cloud image SHA-256 from the image provider.

```powershell
.\scripts\windows\03-Init-HyperV-Runtime.ps1 `
  -PystackExe .\target\release\pystack.exe `
  -Sha256 "<official-image-sha256>" `
  -Timeout 900
```

The init flow:

```text
1. Ensures SSH key
2. Downloads cloud image
3. Verifies SHA-256
4. Converts image to dynamic VHDX if needed
5. Generates cloud-init seed ISO
6. Creates Hyper-V VM
7. Boots VM
8. Discovers IP
9. Waits for SSH
10. Bootstraps containerd + nerdctl
11. Runs runtime health check
```

## Phase 5: smoke test

```powershell
.\scripts\windows\04-Smoke-Test.ps1 -PystackExe .\target\release\pystack.exe
```

Then open:

```text
http://127.0.0.1:18080
```

## Phase 6: optional local Docker API task

This installs a logon scheduled task for the local Docker API shim on `127.0.0.1:23750`.

```powershell
.\scripts\windows\05-Install-Api-Task.ps1 -PystackExe "$(Resolve-Path .\target\release\pystack.exe)"
```

The API requires a bearer token saved under:

```text
C:\ProgramData\PyStack\config\docker-api-token.txt
```

## Phase 7: diagnostics

```powershell
.\scripts\windows\06-Diagnostics.ps1 `
  -PystackExe .\target\release\pystack.exe `
  -Config stack.json `
  -Output .pystack\diagnostics `
  -Tail 200
```

## One-command phased deploy

```powershell
.\scripts\windows\Invoke-PyStackLocalDeploy.ps1 `
  -Phase All `
  -Sha256 "<official-image-sha256>" `
  -SmbPassword "<your-windows-password>" `
  -SkipDesktop
```

## Production MVP acceptance checklist

Do not call the local deployment successful until all pass:

```powershell
cargo fmt --check
cargo test --workspace --locked
cargo build --workspace --locked
.\target\release\pystack.exe hyperv doctor
.\target\release\pystack.exe hyperv health
.\scripts\windows\04-Smoke-Test.ps1 -PystackExe .\target\release\pystack.exe
.\scripts\windows\06-Diagnostics.ps1 -PystackExe .\target\release\pystack.exe
```

## Known MVP limitations

```text
UDP port publishing is unsupported.
SMB credentials are not DPAPI-protected yet.
The optional Docker API shim targets supported workflows, not full universal Docker Engine parity.
Kubernetes/CRI support is not included.
Production multi-user/shared-server deployment is not recommended before DPAPI/Credential Manager integration.
```
