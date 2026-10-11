# StackDeck / StackDeck Hyper-V Container Runtime

Release candidates, signing requirements, clean-client qualification, and final publication are governed by [RELEASE_GATES.md](RELEASE_GATES.md). Passing source/build tests alone is not permission to publish an unsigned or unqualified Windows installer.

This repository is a **Rust-first Windows container orchestration project**. It is not a Python package and it does not ship a Python zipapp. The main deliverables are:

- `stackdeck-cli` / `stackdeck-hive`: Rust CLI binaries
- `stackdeck-hyperv`: Hyper-V backend crate
- `stackdeck-api`: local Docker Engine API compatibility shim backed by the Hyper-V VM
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

## Docker API compatibility contract

The local API is loopback-only and requires
`Authorization: Bearer <STACKDECK_DOCKER_API_TOKEN>` by default, including on
loopback. The scheduled-task installer creates a private token file and injects
the token into the daemon process. This closes the unauthenticated-local-daemon
boundary; direct Docker clients that cannot attach the header need an
authenticated adapter. `STACKDECK_DOCKER_API_REQUIRE_TOKEN=0` is an explicit,
compatibility-only opt-out and is not a release-safe default. A non-loopback
listener additionally requires `--allow-remote` and must keep token
authentication enabled.

Published host ports are currently **TCP-only**. Docker API and Compose requests
that publish UDP are rejected before runtime or state mutation; Windows
`netsh interface portproxy v4tov4` cannot safely relay UDP. UDP is not silently
converted to TCP.


## Local deployment package

For the supported client baseline (**Windows 11 Pro/Enterprise/Education x86-64 with Hyper-V**), start here. Windows 10 must be explicitly qualified for the client estate and passed with `-AllowWindows10` in the prerequisite script:

```powershell
Set-ExecutionPolicy -Scope Process Bypass -Force
.\scripts\windows\00-Ensure-Prereqs.ps1
.\scripts\windows\01-Build-Release.ps1 -SkipDesktop
.\scripts\windows\02-Configure-HyperV.ps1 -StackDeckExe .\target\release\stackdeck-cli.exe -SkipSmb
.\scripts\windows\03-Init-HyperV-Runtime.ps1 -StackDeckExe .\target\release\stackdeck-cli.exe
.\scripts\windows\04-Smoke-Test.ps1 -StackDeckExe .\target\release\stackdeck-cli.exe
```

Detailed phase-by-phase guide: `docs/LOCAL_DEPLOYMENT_WINDOWS.md`.

## Build

```powershell
cargo fmt --check
cargo test -p stackdeck-api -p stackdeck-hyperv -p stackdeck-compose -p stackdeck-types -p stackdeck-state -p stackdeck-process --locked
cargo build -p stackdeck-cli --bin stackdeck-cli --release --locked
cargo build -p stackdeck-cli --bin stackdeck-hive --release --locked
```

Desktop payload build (unsigned developer validation only):

```powershell
.\scripts\windows\01-Build-Release.ps1
```

A publishable Windows release must use the protected signed-release workflow or
`scripts\windows\09-Build-Signed-Release.ps1`. The release sequence is
**build payloads -> sign payloads -> bundle MSI/NSIS -> sign installers ->
extract/verify embedded signatures -> checksums/provenance/attestation**.
`npm run package:windows:unsigned` exists only for local packaging diagnostics
and must never be published.

Do not run `cargo build -p stackdeck` and then `tauri bundle`: a direct Cargo
build uses Tauri's development URL and produces an installer that tries to open
`127.0.0.1:1420`. `package:windows` performs the Tauri release build, confirms
that the current hashed Vite asset is embedded, and only then creates MSI/NSIS.

The desktop package stages the sidecar as both:

```text
desktop/stackdeck/src-tauri/bin/stackdeck-hive-x86_64-pc-windows-msvc.exe
desktop/stackdeck/src-tauri/bin/stackdeck-hive.exe
```

Generated binaries are release artifacts and should not be committed.

### Installed desktop first-run setup

The NSIS/MSI installs only the signed application payload under Program Files.
Creating a Hyper-V VM during MSI execution would make install, repair, upgrade,
and uninstall destructive and difficult to recover. On first launch, StackDeck
therefore opens the **VM** page when no VHDX is present and asks for:

- the local VM folder (for example `C:\StackDeck\VMs\stackdeck-linux`);
- a dynamic VHDX capacity from 8 to 2048 GB.

Selecting **Create and initialize runtime** shows one Windows UAC prompt, obtains
the official Debian SHA-512 manifest, creates the dynamic VHDX in the selected
folder, initializes the VM, and bootstraps containerd. SMB is disabled by
default, so no Windows password is requested. Host bind mounts can be enabled
separately after installation.

Before publishing installers, bump the release version and run the protected
`.github/workflows/release-windows.yml` workflow. It signs the desktop payload
and staged sidecar **before** Tauri bundles them, then signs MSI/NSIS, extracts
both installers to verify their embedded Authenticode payloads, writes
SHA-256/provenance records, and creates a GitHub build-provenance attestation.
Unsigned local packages are diagnostics only and are not public-release ready.

## First-time Hyper-V setup

The following is the CLI/manual alternative to the desktop first-run flow. Run
PowerShell as Administrator.

```powershell
Enable-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All -All -NoRestart
Enable-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-Management-PowerShell -All -NoRestart
```

Restart Windows if Hyper-V was newly enabled.

Create a VM root on a drive with enough free space with a small 10 GB dynamic VHDX target:

```powershell
New-Item -ItemType Directory -Force -Path "$env:SystemDrive\StackDeck\VMs\stackdeck-linux" | Out-Null
New-Item -ItemType Directory -Force -Path "$env:SystemDrive\StackDeck\Images" | Out-Null

stackdeck-cli hyperv configure `
  --vm-name "stackdeck-linux" `
  --vm-root "$env:SystemDrive\StackDeck\VMs\stackdeck-linux" `
  --disk-gb 10 `
  --memory-mb 4096 `
  --cpus 2 `
  --switch-name "Default Switch" `
  --portproxy true
```

Generate or confirm the SSH key:

```powershell
stackdeck-cli hyperv ensure-key
```

Initialize from the release-qualified runtime lock. Public/release builds fail
closed until `runtime\windows-runtime-lock.json` contains a pinned Debian image
URL (never `/latest/`), its SHA-512, a Debian snapshot timestamp, and the pinned
nerdctl version/archive SHA-256. Release engineering writes this file with
`scripts\windows\12-Lock-Runtime-Image.ps1`; normal clients do not resolve a
live `bookworm/latest` manifest at install time.

```powershell
stackdeck-cli hyperv init --timeout 900
```

A custom image remains supported only when the operator supplies its exact
HTTPS URL and trusted SHA-256/SHA-512 explicitly.

The init flow downloads the image, verifies SHA-512 or SHA-256 before promotion, converts to dynamic VHDX when needed, creates cloud-init seed ISO, creates the Hyper-V VM, discovers IP, waits for SSH, and bootstraps containerd/nerdctl.

## SMB live bind mounts

Live host bind mounts require SMB config. The Hyper-V backend no longer silently copies bind mounts with SCP. If a service uses a bind mount and SMB is not configured, startup fails with a configuration error.

For a VM without host bind mounts, configure/deploy with `-SkipSmb`; this avoids
storing unusable credentials and leaves SMB unavailable until configured later.

Configure SMB access. The deployment script is preferred because it accepts the
password as a `SecureString`:

```powershell
.\scripts\windows\02-Configure-HyperV.ps1 `
  -StackDeckExe .\target\release\stackdeck-cli.exe `
  -SmbPassword (Read-Host "Windows SMB password" -AsSecureString)
```

The deployment helper streams the decrypted `SecureString` to the child CLI over
stdin, so the password is not placed in the child process command line. For
automation that calls the CLI directly, use the stdin-only path as well:

```powershell
$secret = Read-Host "Windows SMB password" -AsSecureString
$bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secret)
try {
  $plain = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
  $plain | stackdeck-cli hyperv configure `
    --windows-host "<Windows-hostname-or-IP>" `
    --smb-user "<WindowsUser>" `
    --smb-password-stdin
} finally {
  if ($bstr -ne [IntPtr]::Zero) { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) }
}
```

The legacy `--smb-password` argument remains for backward compatibility but
should not be used in production because process command lines may be observable.

Create or mount shares:

```powershell
stackdeck-cli hyperv share add --path "D:\Projects\my-app" --name my-app
stackdeck-cli hyperv share mount --name my-app
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
stackdeck-cli compose up -f docker-compose.yml --backend hyperv -d
```

Supervise services:

```powershell
stackdeck-cli compose up -f docker-compose.yml --backend hyperv --supervise --interval 10
```

For Hyper-V services with healthchecks, supervision treats non-healthy containers as bad and restarts them unless `restart: no`.

## Diagnostics bundle

Generate a redacted diagnostics directory bundle:

```powershell
stackdeck-cli diagnostics --config stack.json --output .stackdeck\diagnostics --tail 200
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
stackdeck-cli diagnostics --config stack.json --output diagnostics.txt --text
```

## Docker API shim

The shim listens on localhost by default and is backed by the Hyper-V runtime, not host Docker Engine:

```powershell
stackdeck-cli daemon serve --host 127.0.0.1 --port 23750
```

Authentication is required unless explicitly running in insecure local mode through the configured CLI/API option.

## Release-qualified Windows client contract

The NSIS package is explicitly **per-machine** and aligns with the MSI/installed
tooling under `Program Files\StackDeck`. StackDeck first-run intentionally
requires the interactive Windows user to already be a local administrator so
UAC elevation preserves the same release operator identity and the private
`C:\ProgramData\StackDeck\config` ACL. All release-side config, SSH keys, API
tokens, and SMB credential blobs use that machine-scoped config root.

Before any release build, create the immutable runtime lock, then verify the
source contract:

```powershell
.\scripts\windows\12-Lock-Runtime-Image.ps1 `
  -ImageUrl "<pinned-debian-image-url>" `
  -ImageSha512 "<128-hex-sha512>" `
  -DebianSnapshotTimestamp "<YYYYMMDDTHHMMSSZ>" `
  -NerdctlVersion "<qualified-version>" `
  -NerdctlSha256Amd64 "<64-hex-sha256>"

.\scripts\windows\13-Verify-Source-Release.ps1
```

Create source handovers only with `14-Package-Source-Handover.ps1`; it uses
forced enumeration so `.github` and `.gitignore` cannot silently disappear and
reopens the archive to enforce their presence.

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

## SMB credential protection

When SMB bind mounts are enabled, StackDeck no longer serializes the SMB
password into `hyperv.json`. The credential is stored separately as a
LocalMachine Windows DPAPI-protected blob (`smb-password.dpapi`) under
`C:\ProgramData\StackDeck\config`; the private file ACL is the access boundary.
Legacy plaintext configs are migrated on load and rewritten without the
password field.

## Local-GO package builder

For a complete unsigned local MSI/NSIS evaluation bundle from the hardened source, run `scripts\windows\15-Build-Local-Go-Package.ps1` on the supported Windows/Hyper-V build host. See `LOCAL_GO_README.md`. Publishable releases must continue to use the signed release workflow.
