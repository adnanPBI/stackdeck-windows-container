# AGENTS.md

Guidance for AI agents working in this repository.

## Project Shape

Rust workspace implementing a WSL-free, Docker-Desktop-free, Hyper-V-backed
local container runtime for Windows with a Docker-compatible CLI/API.
Deliverables:

- `stackdeck-cli` / `stackdeck-hive`: Rust CLI binaries (`crates/stackdeck-cli`;
  both `[[bin]]` targets share `src/main.rs`)
- `stackdeck-hyperv`: Hyper-V backend for VM lifecycle, SSH/nerdctl
  orchestration, portproxy, and SMB
- `stackdeck-api`: local Docker Engine API compatibility shim (axum)
- `desktop/stackdeck`: Tauri 2 + React desktop UI (optional; needs Node assets
  to build)

Runtime architecture:

```text
Windows host -> Rust CLI/Tauri -> Hyper-V Linux VM -> containerd + nerdctl -> OCI containers
```

The runtime is intentionally independent of WSL, Docker Desktop, and any host
Docker Engine.

## Build & Test Verification

Run these from the repo root after code changes before declaring work done:

```powershell
cargo fmt --check
cargo test -p stackdeck-api -p stackdeck-hyperv -p stackdeck-compose -p stackdeck-types -p stackdeck-state -p stackdeck-process
cargo build -p stackdeck-cli --bin stackdeck-cli --release
cargo build -p stackdeck-cli --bin stackdeck-hive --release
```

Expected: `cargo fmt --check` exits 0; tests report `test result: ok` across
the 6 core crates; both binaries produce `target\release\stackdeck-cli.exe` and
`target\release\stackdeck-hive.exe`.

### Why Tests Are Scoped, Not `--workspace`

`cargo build/test --workspace` also compiles `desktop/stackdeck/src-tauri`,
which depends on Tauri and requires the built frontend (`dist/`) plus WebView2
tooling. That path is only needed for the full desktop package. For routine
code verification, use the scoped commands above; they cover all CLI/API/runtime
logic without pulling the Tauri frontend step. `scripts/windows/01-Build-Release.ps1`
intentionally uses this scoped core suite and builds the desktop payload
separately.

### stackdeck-api Auth Gotcha

The Docker API shim enforces a mandatory bearer-token middleware: every request
needs `Authorization: Bearer <STACKDECK_DOCKER_API_TOKEN>`, and `serve()`
refuses to start without the token set. HTTP tests must send the header. Use the
test-local `authed(request_builder)` helper and `isolated_home()` (which seeds a
token) in `crates/stackdeck-api/src/lib.rs`. New HTTP route tests that forget
the header will fail with `401` instead of the expected status.

## Deployment Phases

The end-to-end runtime cannot be brought up from a non-administrator session.
Phases 3-7 need Administrator PowerShell, Hyper-V enabled, and real host inputs.
Ordered runbook:

```powershell
Set-ExecutionPolicy -Scope Process Bypass -Force
cd <repo-root>
.\scripts\windows\00-Ensure-Prereqs.ps1 -EnableFeatures   # reboot if Hyper-V was newly enabled
.\scripts\windows\01-Build-Release.ps1 -SkipDesktop        # CLI-only build
.\scripts\windows\02-Configure-HyperV.ps1 -StackDeckExe .\target\release\stackdeck-cli.exe -SkipSmb
.\scripts\windows\03-Init-HyperV-Runtime.ps1 -StackDeckExe .\target\release\stackdeck-cli.exe -Timeout 900
.\scripts\windows\04-Smoke-Test.ps1 -StackDeckExe .\target\release\stackdeck-cli.exe
.\scripts\windows\05-Install-Api-Task.ps1 -StackDeckExe "$(Resolve-Path .\target\release\stackdeck-cli.exe)"
.\scripts\windows\06-Diagnostics.ps1 -StackDeckExe .\target\release\stackdeck-cli.exe -Config stack.json -Output .stackdeck\diagnostics -Tail 200
```

One-command path without host bind mounts:

```powershell
.\scripts\windows\Invoke-StackDeckLocalDeploy.ps1 -Phase All -SkipSmb -SkipDesktop
```

If SMB bind mounts are required, pass:

```powershell
-SmbPassword (Read-Host "Windows SMB password" -AsSecureString)
```

`-Sha512` is optional only for the default Debian image because the init script
resolves Debian's official manifest. Custom image URLs still require `-Sha256`
or `-Sha512`.

### Hard Prerequisites For The Runtime

- Administrator PowerShell for VM creation, `netsh portproxy`, SMB shares, and
  highest-run-level scheduled tasks.
- Hyper-V enabled (`Microsoft-Hyper-V-All`); `Get-VM`/`Resize-VHD` cmdlets
  present.
- `qemu-img` for the default Debian `.qcow2` path. The backend also checks
  `C:\Program Files\qemu\qemu-img.exe` when it is not on `PATH`. A
  pre-converted `.vhdx` passed via `--image-vhdx` avoids conversion.
- `oscdimg` (Windows ADK) or `genisoimage` are optional accelerators for the
  cloud-init seed ISO. If neither exists, the backend falls back to Windows
  IMAPI2 and still creates the seed ISO.
- Official Debian cloud-image SHA-512 is resolved automatically by the init
  script for the default image. Custom downloaded images require an explicit
  trusted SHA-256 or SHA-512. SMB credentials are needed only when host bind
  mounts are enabled.

### Security Posture

- SMB passwords must not be serialized into Hyper-V JSON. The backend stores
  them separately as a LocalMachine Windows DPAPI-protected blob under
  `C:\ProgramData\StackDeck\config` (or `STACKDECK_CONFIG_DIR`) and migrates
  legacy plaintext JSON on load. The private machine config ACL is the access
  boundary.
- Docker API shim must stay bound to loopback unless explicitly run with
  `--allow-remote` behind a hardened boundary.
- Diagnostics output is best-effort redacted; operators remain responsible for
  what leaves the host.
