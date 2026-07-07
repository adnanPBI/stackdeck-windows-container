# AGENTS.md

Guidance for AI agents working in this repository.

## Project shape

Rust workspace implementing a **WSL-free, Docker-Desktop-free, Hyper-V-backed local container runtime for Windows** with a Docker-compatible CLI/API. Deliverables:

- `pystack` / `hive` — Rust CLI binaries (`crates/pystack-cli`, both `[[bin]]` targets share `src/main.rs`)
- `pystack-hyperv` — Hyper-V backend (VM lifecycle, SSH/nerdctl orchestration, portproxy, SMB)
- `pystack-api` — local Docker Engine API compatibility shim (axum)
- `desktop/stackdeck` — Tauri 2 + React desktop UI (optional; needs Node assets to build)

Runtime architecture: `Windows host -> Rust CLI/Tauri -> Hyper-V Linux VM -> containerd + nerdctl -> OCI containers`. Intentionally independent of WSL, Docker Desktop, and any host Docker Engine.

## Build & test verification (run these after every code change)

These are the exact commands that validate the source. Run them from the repo root with Bash before declaring work done. All must pass.

```powershell
# 1. Formatting gate (also runs first in scripts/windows/01-Build-Release.ps1)
cargo fmt --check

# 2. Core unit tests (scoped — see note below on why not --workspace)
cargo test -p pystack-api -p pystack-hyperv -p pystack-compose -p pystack-types -p pystack-state -p pystack-process

# 3. Release CLI binaries (the actual deliverables)
cargo build -p pystack-cli --bin pystack --release
cargo build -p pystack-cli --bin hive --release
```

Expected: `cargo fmt --check` exits 0; tests report `test result: ok` (66 tests across the 6 core crates); both binaries produce `target\release\pystack.exe` and `target\release\hive.exe`.

### Why tests are scoped, not `--workspace`

`cargo build/test --workspace` (used by `scripts/windows/01-Build-Release.ps1`) also compiles `desktop/stackdeck/src-tauri`, which depends on Tauri and requires the built frontend (`dist/`) plus WebView2 tooling. That path is only needed for the full desktop package. For routine code verification, use the scoped commands above — they cover all CLI/API/runtime logic without pulling the Tauri frontend step.

### pystack-api auth gotcha

The Docker API shim enforces a mandatory bearer-token middleware: every request needs `Authorization: Bearer <PYSTACK_DOCKER_API_TOKEN>`, and `serve()` refuses to start without the token set. HTTP tests must send the header — use the test-local `authed(request_builder)` helper and `isolated_home()` (which seeds a token) in `crates/pystack-api/src/lib.rs`. New HTTP route tests that forget the header will fail with `401` instead of the expected status.

## Deployment phases (require an elevated Windows shell)

The end-to-end runtime cannot be brought up from a non-administrator session. Phases 3–7 need Administrator PowerShell, Hyper-V enabled, and real host inputs. Ordered runbook:

```powershell
Set-ExecutionPolicy -Scope Process Bypass -Force
cd <repo-root>
.\scripts\windows\00-Ensure-Prereqs.ps1 -EnableFeatures   # reboot if Hyper-V was newly enabled
.\scripts\windows\01-Build-Release.ps1 -SkipDesktop        # CLI-only build (or drop -SkipDesktop for full desktop)
.\scripts\windows\02-Configure-HyperV.ps1 -PystackExe .\target\release\pystack.exe -SmbPassword "<windows-password>"
.\scripts\windows\03-Init-HyperV-Runtime.ps1 -PystackExe .\target\release\pystack.exe -Sha256 "<official-ubuntu-sha256>" -Timeout 900
.\scripts\windows\04-Smoke-Test.ps1 -PystackExe .\target\release\pystack.exe   # then open http://127.0.0.1:18080
.\scripts\windows\05-Install-Api-Task.ps1 -PystackExe "$(Resolve-Path .\target\release\pystack.exe)"
.\scripts\windows\06-Diagnostics.ps1 -PystackExe .\target\release\pystack.exe -Config stack.json -Output .pystack\diagnostics -Tail 200
```

One-command path: `.\scripts\windows\Invoke-PyStackLocalDeploy.ps1 -Phase All -Sha256 "<sha256>" -SmbPassword "<password>" -SkipDesktop`.

### Hard prerequisites for the runtime (not for build/test)

- Administrator PowerShell (VM creation, `netsh portproxy`, SMB shares, Highest-run-level scheduled task).
- Hyper-V enabled (`Microsoft-Hyper-V-All`); `Get-VM`/`Resize-VHD` cmdlets present.
- **`oscdimg` (Windows ADK) OR `genisoimage`** — required by Phase 4 to build the cloud-init seed ISO. Both are commonly missing; install one before `03-Init-HyperV-Runtime.ps1`.
- `qemu-img` — only needed if the Ubuntu cloud image is qcow2/raw and must be converted to VHDX. A pre-converted `.vhdx` passed via `--image-vhdx` avoids this.
- Official Ubuntu cloud-image SHA-256 and the operator's Windows SMB password.

### Security posture (do not change without thought)

- SMB credentials are stored in **plaintext** in the Hyper-V config (`stack.json` / registry dir) by design for local-only single-developer use. This is documented in `SECURITY_REVIEW.md` and `README.md`. Do not wire DPAPI/Credential Manager without coordinating — it would change the documented local-only contract.
- Docker API shim must stay bound to loopback unless explicitly run with `--allow-remote` behind a hardened boundary.
- Diagnostics output is best-effort redacted; operators remain responsible for what leaves the host.
