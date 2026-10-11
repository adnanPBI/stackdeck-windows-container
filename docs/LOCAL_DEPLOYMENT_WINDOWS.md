# Local deployment guide: Windows 11 Hyper-V runtime

This guide turns the repository into a local Windows deployment for the production MVP architecture:

```text
CLI / StackDeck UI
  -> local StackDeck runtime commands / optional local API task
  -> Hyper-V Linux VM
  -> containerd + nerdctl inside VM
  -> OCI containers
```

## Non-goals for this MVP

- No WSL or WSL2.
- No Docker Desktop.
- No host Docker Engine or host `docker.exe` dependency.
- No UDP port publishing yet. TCP publishing is supported; UDP must fail clearly.
- SMB credentials are persisted separately from JSON with LocalMachine Windows DPAPI protection under `C:\ProgramData\StackDeck\config`.

## Phase 0: unpack and open Admin PowerShell

Run every install/init step in **Administrator PowerShell**.

For an installed NSIS/MSI desktop build, these repository phases are not the
normal end-user path. Launch StackDeck, open **VM**, choose the local VM folder
and dynamic disk capacity, and select **Create and initialize runtime**. The app
performs the equivalent configure/init sequence behind one UAC prompt and does
not request SMB credentials by default.

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

Supported baseline:

```text
Windows 11 Pro/Enterprise/Education x86-64
Hardware virtualization + SLAT
Hyper-V enabled with the Default Switch
```

Windows 10 is not part of the default release baseline. If a client has a
separately qualified ESU/LTSC estate, rerun prerequisites with
`-AllowWindows10` and keep that qualification evidence with the release.

Required tools:

```text
Hyper-V PowerShell module
OpenSSH ssh/scp/ssh-keygen
tar
Rust cargo/rustc
qemu-img unless using preconverted VHDX
oscdimg or genisoimage are optional cloud-init seed ISO accelerators
node/npm only for desktop package build
```

## Phase 2: build release payloads

```powershell
.\scripts\windows\01-Build-Release.ps1
```

This intentionally stops before MSI/NSIS creation. A publishable release must
use `.\scripts\windows\09-Build-Signed-Release.ps1` (normally from the
protected GitHub Actions workflow) so payloads are signed before bundling.

For CLI-only deployment:

```powershell
.\scripts\windows\01-Build-Release.ps1 -SkipDesktop
```

Release binaries:

```text
target\release\stackdeck-cli.exe
target\release\stackdeck-hive.exe
```

## Phase 3: configure Hyper-V runtime on the system drive

```powershell
.\scripts\windows\02-Configure-HyperV.ps1 `
  -StackDeckExe .\target\release\stackdeck-cli.exe `
  -VmRoot "$env:SystemDrive\StackDeck\VMs\stackdeck-linux" `
  -DiskGB 10 `
  -MemoryMB 4096 `
  -Cpus 2 `
  -SwitchName "Default Switch" `
  -WindowsHost $env:COMPUTERNAME `
  -SmbUser "$env:USERDOMAIN\$env:USERNAME" `
  -SmbPassword (Read-Host "Windows SMB password" -AsSecureString)
```

If live Windows bind mounts are not needed yet, omit SMB credentials and use
`-SkipSmb`. The VM, OCI runtime, Docker API, and named volumes work normally;
Compose services with host bind mounts will fail closed until SMB is configured.

The default VHDX is a small 10 GB virtual disk, and 8 GB is the supported minimum. Hyper-V dynamic VHDX grows physically only as data is written, up to its configured virtual size. Expansion requires explicit `Resize-VHD` plus a Linux filesystem grow or the app-managed disk reconciler.

## Phase 4: initialize the VM

The default runtime is loaded from `runtime\windows-runtime-lock.json`. A release build is blocked until this lock contains a pinned Debian image URL (not `/latest/`), image SHA-512, Debian snapshot timestamp, and exact nerdctl archive SHA-256. Custom download URLs still require an explicit SHA-256 or SHA-512.

```powershell
.\scripts\windows\03-Init-HyperV-Runtime.ps1 `
  -StackDeckExe .\target\release\stackdeck-cli.exe `
  -Timeout 900
```

The init flow:

```text
1. Ensures SSH key
2. Downloads cloud image
3. Verifies SHA-512 or SHA-256 before promotion
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
.\scripts\windows\04-Smoke-Test.ps1 -StackDeckExe .\target\release\stackdeck-cli.exe
```

Then open:

```text
http://127.0.0.1:18080
```

## Phase 6: optional local Docker API task

This installs a logon scheduled task for the local Docker API shim on `127.0.0.1:23750`.

```powershell
.\scripts\windows\05-Install-Api-Task.ps1 -StackDeckExe "$(Resolve-Path .\target\release\stackdeck-cli.exe)"
```

The API requires a bearer token saved under:

```text
C:\ProgramData\StackDeck\config\docker-api-token.txt
```

## Phase 7: diagnostics

```powershell
.\scripts\windows\06-Diagnostics.ps1 `
  -StackDeckExe .\target\release\stackdeck-cli.exe `
  -Config stack.json `
  -Output .stackdeck\diagnostics `
  -Tail 200
```

## One-command phased deploy

```powershell
.\scripts\windows\Invoke-StackDeckLocalDeploy.ps1 `
  -Phase All `
  -SmbPassword (Read-Host "Windows SMB password" -AsSecureString) `
  -SkipDesktop
```

## Production MVP acceptance checklist

Do not call the local deployment successful until all pass:

```powershell
cargo fmt --check
cargo test -p stackdeck-api -p stackdeck-hyperv -p stackdeck-compose -p stackdeck-types -p stackdeck-state -p stackdeck-process --locked
cargo build -p stackdeck-cli --bin stackdeck-cli --release --locked
cargo build -p stackdeck-cli --bin stackdeck-hive --release --locked
.\target\release\stackdeck-cli.exe hyperv doctor
.\target\release\stackdeck-cli.exe hyperv health
.\target\release\stackdeck-cli.exe hyperv ps
.\scripts\windows\04-Smoke-Test.ps1 -StackDeckExe .\target\release\stackdeck-cli.exe
.\scripts\windows\06-Diagnostics.ps1 -StackDeckExe .\target\release\stackdeck-cli.exe
```

## Known MVP limitations

```text
UDP port publishing is unsupported.
SMB credentials are LocalMachine DPAPI-protected under `C:\ProgramData\StackDeck\config`; access is controlled by the private StackDeck ACL.
The optional Docker API shim targets supported workflows, not full universal Docker Engine parity.
Kubernetes/CRI support is not included.
Production multi-user/shared-server deployment is not recommended before a separately reviewed enterprise credential-management policy.
```

## Clean-machine installed-client qualification

After installing MSI/NSIS and completing desktop first-run VM setup, run the
qualification gate from an Administrator PowerShell using the matching release
source/scripts. Pre-sign functional qualification may use `-SkipSignature`; the
final release gate must not use that switch:

```powershell
.\scripts\windows\11-Verify-Installed-Client.ps1 `
  -Output .\stackdeck-client-qualification.json
```

Pre-sign functional-only run:

```powershell
.\scripts\windows\11-Verify-Installed-Client.ps1 `
  -SkipSignature `
  -Output .\stackdeck-client-functional-qualification.json
```

For a separately approved Windows 10 ESU/LTSC estate, add `-AllowWindows10`.
Archive the resulting JSON beside the signed installer hashes, provenance, and
GitHub attestation. A client release is not considered qualified until this gate
and the project smoke/restart/recovery matrix pass on a clean supported host.
