# MEMORY.md — Running Journal

> **Living document.** This is the agent's durable memory across sessions. Append (do not rewrite history) at the end of every work session, before context cleanup/handoff. Newest entries on top within each section. See [Maintenance Instructions](#maintenance-instructions) at the bottom.

**Repo root:** `D:\Coding Development Files\StackDeck-HyperV-Local-Container`
**Git baseline:** current checked-out commit `d0ba672`; the worktree contains the uncommitted release-remediation overlay and must not be described as clean or committed.
**Target:** Windows host, Hyper-V-backed, WSL-free / Docker-Desktop-free container runtime with Docker-compatible CLI/API.

---

## Quick Status (2026-08-10)

| Item | State |
|------|-------|
| Source gates | ✅ v8 r2 source contract, PowerShell parser, workflow YAML parse, Rust 1.88 formatting, 104 scoped core tests, locked release CLI/hive builds, desktop frontend build, and Tauri Rust check passed |
| Desktop production build | ✅ Desktop Rust crate and frontend compile; Tauri check requires temporary sidecar staging, and generated sidecar binaries were removed afterward |
| Installer artifacts | ⚠️ release scripts are ready for signed MSI/NSIS production; no final signed client installers were produced locally in this session |
| Source handover packaging | ✅ `14-Package-Source-Handover.ps1` is Git-manifest driven and validates `.github`, `.gitignore`, runtime lock, and release scripts; commit required files before using default mode |
| Installed application | ⚠️ Any existing Program Files install may predate current v8 r2 hardening; reinstall only from a newly signed/qualified 0.6.0 package |
| Runtime state model | ✅ release config/keys/tokens/SMB credential blob default to `C:\ProgramData\StackDeck\config` with private ACLs and `STACKDECK_CONFIG_DIR` override support |
| Runtime lock | ✅ populated with dated Debian Bookworm image, SHA-512, Debian snapshot timestamp, nerdctl-full 2.3.5, and archive SHA-256 |
| VHDX/VM on this host | ⏳ not requalified in this session; live Hyper-V VM boot/bootstrap/container workload evidence remains external |
| Signing/public release | ❌ blocked until protected Authenticode signing, CI attestations, installer payload verification, and clean-machine/live Hyper-V qualification complete |
| Repository state | ⚠️ dirty worktree at `d0ba672`; v8 r2 remediation overlay and generated release docs/scripts are not yet committed |

---

## 2026-08-10 v8 r2 Release-Readiness Handoff

This is the current authoritative handoff. The v8 r2 patch archive was inspected
as an overlay, applied where files were still at the audited baseline or
missing, and then extended where the patch did not fully meet the stronger
release plan.

### Applied remediation

- Applied the r2 payload files from
  `StackDeck-HyperV-Local-Container-8-patched-scripts-v0.6.0-r2.zip`, excluding
  a blind workflow overwrite.
- Manually merged `.github/workflows/release-windows.yml` so it keeps the prior
  pinned-action/certificate-cleanup/tag-publication hardening while adding r2's
  runtime-lock materialization and pre-bundle payload manifest publication.
- Added `runtime/windows-runtime-lock.json` and populated it with release-pinned
  inputs:
  - Debian image:
    `https://cloud.debian.org/images/cloud/bookworm/20260806-2562/debian-12-genericcloud-amd64-20260806-2562.qcow2`
  - Debian image SHA-512:
    `3622c990108a044ed411652f8741e77c5822c365114d7b940206b243f8fb617b8586792df4cdb7afba1b71d1a09289d8ed632124688f2c8352cb08190a1e9868`
  - Debian snapshot timestamp: `20260806T120000Z`
  - nerdctl full archive: `nerdctl-full-2.3.5-linux-amd64.tar.gz`
  - nerdctl full archive SHA-256:
    `b697295c623639734aaab737523c808fd3cc8d3046039fd94fff1744e4c317aa`
- Set Tauri NSIS `installMode` to `perMachine`, aligning installer layout with
  `C:\Program Files\StackDeck` scripts.
- Removed the legacy HTTP `gui` command from the production CLI/hive command
  surface and removed `stackdeck-gui` from the production workspace/dependency
  graph. The source verifier now fails if it is wired back in.
- Moved release config discovery to `C:\ProgramData\StackDeck\config`, with
  `STACKDECK_CONFIG_DIR` as an explicit test/dev override.
- Changed SMB credential persistence to LocalMachine DPAPI and kept the
  credential out of `hyperv.json`. Access control is now the private
  ProgramData ACL, not a per-user profile path.
- Updated elevated scripts and desktop first-run provisioning to initialize the
  ProgramData config root and export `STACKDECK_CONFIG_DIR` before invoking the
  sidecar.
- Replaced hardcoded `D:` installed-runtime defaults with
  `%SystemDrive%\StackDeck\VMs\stackdeck-linux`.
- Added/kept `--smb-password-stdin` and changed the SecureString deployment
  helper path to stream the SMB password to the CLI instead of putting it in
  child-process argv.
- Added `11-Verify-Installed-Client.ps1 -SkipSignature` for pre-sign functional
  qualification while keeping signature checks strict by default.
- Strengthened installer verification with a pre-bundle payload hash manifest
  and exact post-extraction hash checks for embedded desktop/hive payloads.
- Reworked `14-Package-Source-Handover.ps1` to build the ZIP from a Git manifest
  and verify required hidden paths after packaging. In this dirty checkout, use
  `-IncludeUntracked` only for pre-commit local rehearsal; the release path
  should run after all required files are committed.
- Updated `.gitignore` so local ZIPs/source manifests do not become accidental
  handover inputs.
- Corrected `Cargo.lock` after `cargo generate-lockfile` floated Tauri
  transitive crates beyond the pinned `tauri-build = 2.2.0` compatibility set.
  The compatible locked set is:
  `tauri 2.5.1`, `tauri-build 2.2.0`, `tauri-utils 2.4.0`,
  `tauri-runtime 2.6.0`, `tauri-runtime-wry 2.6.0`,
  `tauri-codegen 2.2.0`, `tauri-macros 2.2.0`, `wry 0.51.2`.

### Validation evidence

- `scripts\windows\13-Verify-Source-Release.ps1`: PASS.
- PowerShell parser over `scripts\windows\*.ps1`: PASS.
- `.github\workflows\release-windows.yml` YAML parse: PASS.
- `desktop\stackdeck\src-tauri\tauri.conf.json` JSON parse: PASS.
- LocalMachine DPAPI protect/unprotect smoke test using
  `[Security.Cryptography.ProtectedData]`: PASS.
- `cargo fmt --check`: PASS.
- `cargo test -p stackdeck-api -p stackdeck-hyperv -p stackdeck-compose -p stackdeck-types -p stackdeck-state -p stackdeck-process --locked`: PASS, 104 tests.
- `cargo build -p stackdeck-cli --bin stackdeck-cli --release --locked`: PASS.
- `cargo build -p stackdeck-cli --bin stackdeck-hive --release --locked`: PASS.
- `scripts\windows\01-Build-Release.ps1 -SkipTests -SkipDesktop`: PASS.
- `npm.cmd run build` in `desktop\stackdeck`: PASS.
- `npm.cmd run stage:hive` followed by `cargo check -p stackdeck --locked`:
  PASS. The generated `desktop\stackdeck\src-tauri\bin\*.exe` sidecars were
  deleted afterward and the source verifier passed again.
- Built CLI help no longer shows `gui`; `rg` found no `stackdeck-gui`,
  `stackdeck_gui`, or `Commands::Gui` references in production CLI/workspace
  files.
- `git diff --check`: PASS, with CRLF warnings only.
- The local scheduled task `StackDeck Local Docker API` was stopped during
  release binary rebuilds to avoid file locks and restarted successfully; final
  state: `Running`.

Known non-failing warnings:

- Cargo warns that `crates\stackdeck-cli\src\main.rs` is used by both
  `stackdeck-cli` and `stackdeck-hive` bin targets. This is existing structure,
  not a release failure.
- `git diff --check` reports only line-ending normalization warnings.

### Not claimed yet

- No final Authenticode-signed MSI/NSIS artifacts were produced locally.
- No protected GitHub Actions attestation/provenance run was executed locally.
- No clean Windows client install/repair/upgrade/uninstall matrix was executed.
- No live Hyper-V runtime bootstrap/container/Compose/API smoke was executed
  after the v8 r2 remediation.
- The existing `StackDeck-HyperV-Local-Container.zip` in the repo root is stale
  and must not be used as release media.

### Next plan

1. Normalize the dirty worktree before release:
   - Review all untracked remediation files and stage the intended ones:
     `.github`, `runtime`, `VALIDATION.md`, new StackDeck crates, new scripts,
     docs, and `rust-toolchain.toml`.
   - Keep audit ZIPs and local package ZIPs out of Git unless explicitly needed
     as evidence artifacts.
   - Commit the v8 r2 remediation as one coherent release-readiness commit.

2. Rehearse source handover packaging:
   ```powershell
   .\scripts\windows\14-Package-Source-Handover.ps1 `
     -OutputZip .\target\source-handover\StackDeck-HyperV-Local-Container-0.6.0-source.zip
   ```
   Expected: archive contains `.github/workflows/release-windows.yml`,
   `.gitignore`, `runtime/windows-runtime-lock.json`, and release scripts; the
   generated `.sha256` and `.manifest.sha256` files verify.

3. Run a local unsigned functional desktop package only as a pre-sign smoke:
   ```powershell
   .\scripts\windows\01-Build-Release.ps1 -BundleUnsigned
   .\scripts\windows\11-Verify-Installed-Client.ps1 -SkipSignature `
     -Output .\target\pre-sign-functional-qualification.json
   ```
   Do not publish these unsigned artifacts.

4. Run the protected signed release path:
   ```powershell
   .\scripts\windows\09-Build-Signed-Release.ps1 `
     -CertificateThumbprint <40-hex-thumbprint> `
     -SourceCommit <release-commit-sha>
   ```
   Expected: sidecars and desktop PE are signed before bundling, MSI/NSIS are
   signed after bundling, `PRE_BUNDLE_PAYLOAD_SHA256.json`, `SHA256SUMS.txt`,
   and `provenance.json` are generated, and
   `10-Verify-Installer-Payloads.ps1` passes.

5. Run clean-machine install qualification on the supported matrix:
   - Windows 11 Pro/Enterprise/Education x86-64 baseline.
   - Optional Windows 10 only with explicit `-AllowWindows10` evidence.
   - Verify install, first launch, runtime init, repair, upgrade, uninstall, and
     reinstall behavior.

6. Run live Hyper-V runtime qualification from an elevated shell:
   ```powershell
   .\scripts\windows\08-Initialize-Installed-Runtime.ps1 `
     -HiveExe "C:\Program Files\StackDeck\stackdeck-hive.exe" `
     -Timeout 900
   .\scripts\windows\11-Verify-Installed-Client.ps1 `
     -Output .\target\stackdeck-client-qualification.json
   ```
   Then run Compose/API smoke, restart/DHCP recovery, portproxy cleanup,
   registry mirror add/remove, diagnostics, and rollback scenarios.

7. Capture final evidence in `VALIDATION.md`:
   - Exact OS build and edition.
   - Installer filenames, sizes, SHA-256 hashes, and Authenticode signer.
   - Runtime lock hash and provenance hash.
   - Clean-machine qualification JSON outputs.
   - Live VM VHDX path/type/size and `hyperv health` plus `hyperv ps` outputs.

8. Tag and publish only after the signed workflow succeeds from the release
   commit and the uploaded assets match local verification.

---

## 2026-08-01 Release-Remediation and Installer Handoff

This section is historical. Use the 2026-08-10 v8 r2 handoff above as the current source of truth. Older entries below retain proof from previous checkouts and VMs; they must not be used to claim that the current v8 r2 source has signed installers or live Hyper-V proof.

### StackDeck 0.6.0 namespace migration

- Eliminated the retired product namespace from source files and repository
  paths. All crates/imports, environment variables, state paths, runtime labels,
  scripts, scheduled-task names, API data paths, workflow references, tests,
  and documentation now use StackDeck naming.
- Renamed CLI deliverables to `stackdeck-cli.exe` and `stackdeck-hive.exe` so
  they cannot collide with the Tauri desktop `StackDeck.exe` on Windows.
- Renamed the live API scheduled task to `StackDeck Local Docker API`, moved its
  existing bearer token to `C:\ProgramData\StackDeck`, and verified authenticated
  `/_ping` with HTTP 200. The task currently executes the repository 0.6.0
  sidecar; repoint it to Program Files when a signed 0.6.0 installer is installed.
- Rust 1.88 formatting, 103 scoped tests plus doc tests, both release CLI builds,
  Tauri production verification, MSI/NSIS bundling, and MSI payload extraction
  passed. The local packages remain unsigned and are not public-release ready.

| Canonical 0.6.0 unsigned artifact | Bytes | SHA-256 |
|---|---:|---|
| `target\release\bundle\msi\StackDeck_0.6.0_x64_en-US.msi` | 214,650,880 | `4F526E21A22B5687C59E6AA17460138E3E08FED2958E5DFFFB105680B6B268BA` |
| `target\release\bundle\nsis\StackDeck_0.6.0_x64-setup.exe` | 215,506,726 | `915866A7DF1AF2D9392E05507043CA3E956C4195D524F041C9AE5A7A1B997BBD` |

### Requested deployment contract

- Install the desktop application in `C:\Program Files\StackDeck`.
- Create the runtime at `D:\StackDeck\VMs\stackdeck-linux`.
- Use a dynamically expanding VHDX with an 8 GB maximum size.
- Do not require an SMB password for the default runtime. SMB is opt-in only for Windows-host bind mounts.
- Make first-run provisioning available inside the desktop UI, with an elevation/UAC boundary comparable to Docker Desktop, instead of requiring the user to know the deployment scripts.

### Release-blocker remediation now present in the worktree

- Native-process safety: stored PIDs are revalidated against process identity; force-start does not silently orphan the previous process; termination failures are surfaced and state is not cleared until stop is confirmed.
- Non-destructive service start: repeated Hyper-V/API start paths do not unconditionally run `nerdctl rm -f` and recreate containers.
- Hyper-V correctness: newly generated SSH identity is preserved; BuildKit is installed for Compose builds; registry mirror writes occur with appropriate privileges; SMB guest-shell inputs are validated/quoted; SSH, healthcheck, build and pull paths use deadlines; command output can be streamed.
- Configuration/state safety: API state corruption fails closed; cross-process locking protects state read-modify-write operations; Hyper-V JSON accepts an optional UTF-8 BOM; state/schema updates were hardened; sensitive diagnostic values remain redacted.
- Docker API compatibility: port protocol/host-IP behavior, port inspection, pull authentication, pull progress streaming, and log framing were corrected and regression-tested. Unsupported UDP publication is rejected explicitly rather than being silently changed to TCP.
- Compose/stack normalization: merge keys/anchors, entrypoint behavior, long/short port validation, anonymous volumes, interpolation boundaries, defaults, build data, healthcheck data, restart/backoff/replica fields, and other previously discarded configuration received remediation and tests. See `REMEDIATION-MATRIX.md` for criterion-by-criterion coverage and `VALIDATION.md` for evidence and limitations.
- Windows/release scripts: native exit codes are checked; Node/npm and supported versions are required for full desktop builds; offline WebView2 packaging is enabled; signing support is provided by `scripts\windows\07-Sign-Release.ps1`; the toolchain is pinned to Rust 1.88.

### Follow-up blocker refinements (2026-08-01)

- Force termination now revalidates creation-time/executable identity after the grace delay and immediately before `taskkill /F`.
- Cross-process JSON locks now use stable OS file locks and never age-delete or unlink the lock path. SQLite upsert is one atomic conflict-merge statement with a concurrent two-connection regression test.
- PowerShell/SSH capture drains pipes while the child runs. Pull streaming is bounded before and after the async boundary, preserves records split across chunks, caps partial lines, and cancels the remote pull when the HTTP client disconnects.
- Docker host ports remain backed by real bound listeners from create through guest start; each allocation is added to the current request's reservation set. API state records `runtime_pending` before guest mutation so a crash cannot turn a possible live container into an untracked `runtime_id: null` entry.
- Loopback Docker API bearer authentication is mandatory by default. Unauthenticated TCP now requires the explicit `STACKDECK_DOCKER_API_REQUIRE_TOKEN=0` compatibility override.
- Registry-mirror removal deletes the active guest directory and restarts containerd before committing JSON removal.
- Hyper-V containers carry a configuration-hash label. Changed configuration and explicit build use stop/rename/launch/healthcheck/rollback replacement instead of deleting the previous writable layer before replacement success.
- The signed Windows workflow now signs CLI payloads, then the production desktop PE and staged sidecar before bundling, then MSI/NSIS; it verifies all signatures, writes/rechecks hashes and local provenance, creates GitHub Sigstore/SLSA provenance, and publishes only version-matched tag assets. All external actions are commit-pinned.

### First-run VHDX and desktop refinements

- Added `scripts\windows\08-Initialize-Installed-Runtime.ps1`, an elevated installed-runtime initializer. Its defaults are the preferred `D:` path, 8 GB disk, official Debian SHA-512 manifest verification, and `-SkipSmb`.
- Added desktop commands `runtime_setup_status` and `provision_runtime`, plus a first-run VM setup form with custom local-drive path, 8–2048 GB disk validation, retryable error reporting, and a **Create and initialize runtime** action.
- Provisioning crosses the UAC boundary using encoded PowerShell and the installed, trusted sidecar path rather than constructing an elevated helper in a user-writable location.
- `02-Configure-HyperV.ps1` and `Invoke-StackDeckLocalDeploy.ps1` accept `-SkipSmb`; the backend enforces an 8 GB minimum dynamic VHDX.
- The Debian manifest fetch supports Windows PowerShell 5.1 response shapes. The default flow retrieves the official digest automatically, so users must not pass a placeholder such as `<official Debian image SHA-512>`.

### Desktop packaging incident and resolution

- A manually assembled early 0.5.1 package launched a WebView pointed at the Vite development URL `127.0.0.1:1420`, producing `ERR_CONNECTION_REFUSED`. That artifact was invalid for release and was quarantined with a `.dev-server-broken` suffix.
- Packaging now runs the proper Tauri production frontend build before bundling. `desktop\stackdeck\scripts\verify-release-binary.ps1` parses `dist\index.html`, requires the current hashed JavaScript asset inside the release PE, checks for the runtime-setup command, and verifies the staged sidecar version.
- `desktop\stackdeck\scripts\stage-hive.ps1` now resolves paths relative to `$PSScriptRoot`, eliminating the prior working-directory failure.
- The installed production application was visually rechecked: the dashboard renders, and the VM tab renders the setup form prefilled with `D:\StackDeck\VMs\stackdeck-linux` and `8` GB.
- The VM tab initially remained at “Loading runtime configuration…” because `%USERPROFILE%\.stackdeck_runner\hyperv.json` began with a UTF-8 BOM. The Hyper-V config loader now strips an optional BOM and has a regression test. The installed sidecar subsequently normalized the file and `stackdeck-hive hyperv doctor` exited 0.

### Current installed and runtime state

- Windows uninstall metadata reports StackDeck `0.5.1` at `C:\Program Files\StackDeck\`.
- Installed files include `stackdeck.exe`, `stackdeck-hive.exe`, and the uninstall shortcut. The small installed-file count is expected because Tauri embeds the frontend assets into the desktop executable and the runtime Linux filesystem belongs in the VHDX, not under Program Files.
- The installed desktop executable and hive sidecar were checked against the corresponding final release binaries during the packaging/install verification.
- `stackdeck-hive hyperv doctor` resolves Hyper-V cmdlets, `ssh`, and `scp` and exits successfully. The recorded `admin=false` result reflects the non-elevated diagnostic shell, not an absent UAC workflow.
- There is currently no `.vhdx` beneath `D:\StackDeck\VMs\stackdeck-linux`. Clicking **Create and initialize runtime** reached the elevation boundary, but no provisioning process started afterward, consistent with UAC not being approved or being dismissed. No successful VM creation may be claimed yet.
- SMB is intentionally disabled in the default setup. It should be configured only if Windows-host bind mounts are needed, at which point the operator must provide valid Windows credentials.

### Validation evidence

- Toolchain: Rust `1.88` pinned by `rust-toolchain.toml`.
- `cargo +1.88.0 fmt --check` passed.
- Scoped core tests passed: API 26, Compose 22, Hyper-V 25, Process 15, State 8, Types 7 = **103 tests**, plus doc tests.
- `cargo +1.88.0 build -p stackdeck-cli --bin stackdeck-cli --release --locked` passed.
- `cargo +1.88.0 build -p stackdeck-cli --bin stackdeck-hive --release --locked` passed; the sidecar reports version `0.5.1`.
- `npm.cmd run build` passed.
- Tauri production build (`tauri build --no-bundle --ci`) and the embedded-asset release guard passed.
- PowerShell parser validation passed for 12 Windows/desktop scripts.
- Final MSI administrative extraction confirmed the current desktop executable, hive sidecar, production frontend asset, and runtime setup command.
- `git diff --check` passed; only line-ending warnings were emitted.

### Canonical 0.5.2 unsigned artifacts (do not publish yet)

| Artifact | Bytes | SHA-256 | Signature |
|---|---:|---|---|
| `target\release\bundle\msi\StackDeck_0.5.2_x64_en-US.msi` | 214,646,784 | `81C6E003A8CB083314B1CF0D5A67A7C2C901A2D4E17943AEE23151C34E4CAC98` | `NotSigned` |
| `target\release\bundle\nsis\StackDeck_0.5.2_x64-setup.exe` | 215,502,408 | `C553D7B9B1053D07C0592A45A44D8AB64DE9983F7FA5B7DCDE447214FAB63007` | `NotSigned` |

Older `.dev-server-broken.*`, `.pre-bom-fix.*`, and `.local-webview-hotfix.*` files are quarantine/diagnostic artifacts. They are not release candidates and must not be uploaded. Authenticode signing will change the canonical package hashes, so calculate and publish new checksums only after signing.

### Remaining tasks — immediate live-runtime gate

- [ ] In the StackDeck VM tab, click **Create and initialize runtime** and approve the Windows UAC prompt; alternatively run `scripts\windows\08-Initialize-Installed-Runtime.ps1` from Administrator PowerShell.
- [ ] Confirm `D:\StackDeck\VMs\stackdeck-linux\stackdeck-linux.vhdx` exists and `Get-VHD` reports `VhdType = Dynamic` and an 8 GB maximum.
- [ ] Confirm Hyper-V VM `stackdeck-linux` is running and attached to the intended D-drive VHDX, then verify fresh IP discovery after boot.
- [ ] Verify SSH readiness, `stackdeck-hive hyperv health`, `stackdeck-hive hyperv ps`, containerd, nerdctl, CNI networking, BuildKit, image pull, and image build.
- [ ] Run a Compose smoke service, verify its healthcheck and `http://127.0.0.1:18080`, then verify `compose status`, `compose logs`, repeat `compose up`, and `compose down` without destructive recreation or orphaning.
- [ ] Exercise authenticated Docker API create/start/list/inspect/logs/pull behavior with a standard client, including framing and registry authentication. Install and validate the API scheduled task only if that compatibility surface is required.
- [ ] Restart the VM and prove DHCP/IP recovery, port forwarding, service state, restart limits, and backoff behavior.
- [ ] Stop or remove the VM while offline and verify stale `netsh portproxy` and firewall rules are cleaned up.
- [ ] Test SMB bind mounts separately if needed. Do not claim them supported on this host until valid credentials and a real bind-mount workload pass.

### Remaining tasks — public release gate

- [ ] Obtain a trusted Authenticode certificate and run `scripts\windows\07-Sign-Release.ps1` for `stackdeck-cli.exe`, `stackdeck-hive.exe`, desktop `stackdeck.exe`, MSI, and NSIS artifacts; verify every signature and timestamp chain.
- [ ] Recompute SHA-256 checksums after signing and produce release provenance/SBOM/attestation as required.
- [ ] Install and uninstall both canonical MSI and NSIS packages on clean or sacrificial Windows machines, including upgrade/reinstall behavior and a machine without preinstalled WebView2.
- [ ] Complete a clean-machine Hyper-V enable/reboot/provision/Compose/API rehearsal using only the shipped installer and first-run UI.
- [ ] Confirm SmartScreen/code-signing behavior, least-privilege boundaries, UAC cancellation/retry UX, and failure recovery for interrupted image download/conversion/bootstrap.
- [ ] Review and intentionally commit the dirty remediation worktree. Exclude quarantine installers, extracted MSI trees, downloaded images, VM disks, secrets, and other local proof artifacts.
- [ ] Confirm the GitHub CI/release workflow builds the same guarded production packages, attach only the signed canonical artifacts, and publish release notes that accurately state the remaining compatibility boundaries.

### Resume commands

```powershell
# Administrator PowerShell: perform the pending runtime initialization.
Set-ExecutionPolicy -Scope Process Bypass -Force
cd 'D:\Coding Development Files\StackDeck-HyperV-Local-Container'
.\scripts\windows\08-Initialize-Installed-Runtime.ps1

# Inspect the requested disk and VM after initialization.
Get-VHD 'D:\StackDeck\VMs\stackdeck-linux\stackdeck-linux.vhdx' |
  Format-List Path,VhdType,FileSize,Size
Get-VM -Name 'stackdeck-linux' | Format-List Name,State,Status
& 'C:\Program Files\StackDeck\stackdeck-hive.exe' hyperv health
& 'C:\Program Files\StackDeck\stackdeck-hive.exe' hyperv ps
```

Read `VALIDATION.md` for the most recent command evidence and `REMEDIATION-MATRIX.md` for the release-blocker mapping before making release-readiness claims.

---

## Project Summary

Rust workspace delivering a local Windows container runtime that is deliberately independent of WSL, Docker Desktop, and any host Docker Engine. The CLI/API drive a Hyper-V Linux VM that runs `containerd + nerdctl` to host OCI containers, with `netsh portproxy` + SMB bind mounts bridging host and VM.

Architecture chain: `Windows host → Rust CLI/Tauri desktop → Hyper-V Linux VM → containerd + nerdctl → OCI containers`.

## Current Architecture

**Workspace members (`Cargo.toml`, edition 2021, Rust 1.88 pinned by `rust-toolchain.toml`):**
- `crates/stackdeck-types` — shared config/types (`HyperVConfig`, `HyperVService`, registry paths, redaction hints)
- `crates/stackdeck-compose` — docker-compose.yml parsing → `ComposeProject`
- `crates/stackdeck-state` — state store
- `crates/stackdeck-process` — native process backend (sqlite via `rusqlite` bundled)
- `crates/stackdeck-hyperv` — Hyper-V backend: VM lifecycle (PowerShell), SSH/nerdctl orchestration, cloud-init seed ISO, qcow2→VHDX, portproxy, SMB shares, registry mirrors
- `crates/stackdeck-api` — Docker Engine API shim (axum 0.8), bearer-token auth middleware, loopback-bound
- `crates/stackdeck-gui` — legacy source-only GUI crate; not part of the production workspace or CLI/hive dependency graph after the v8 r2 hardening
- `crates/stackdeck-cli` — `stackdeck-cli` + `stackdeck-hive` bins (both `[[bin]]` → `src/main.rs`)
- `desktop/stackdeck/src-tauri` — Tauri 2 + React desktop UI (needs Node `dist/` + WebView2 to build/package)

**Release profile:** `strip=true`, `opt-level="z"`, `lto=true`, `codegen-units=1` (small binary, slower link).

**Config layout:** release config defaults to `C:\ProgramData\StackDeck\config` on Windows, with `STACKDECK_CONFIG_DIR` available for tests/dev. SMB passwords are excluded from `hyperv.json` and stored as a LocalMachine DPAPI-protected blob guarded by the private StackDeck ACL.

## Implemented Features (existing, pre-session)

- CLI: `init/validate/doctor/up/down/restart/status/logs/diagnostics/watch/register/unregister`, plus Compose, Hyper-V, and daemon subcommands. The legacy HTTP `gui` command is intentionally absent from production CLI/hive.
- Compose: `up/build/down/status/logs` with dependency-condition waiting (`service_started`/`service_healthy`/`service_completed_successfully`), supervise mode
- Hyper-V: `configure/ensure-key/doctor/init/download-image/create-cloud-vm/start/stop/ip/discover-ip/bootstrap/health/repair/ps/exec`, plus `snapshot/image/volume/network/share/mirror` subcommands
- Docker API shim: `/_ping`, `/version`, `/containers/*`, `/images/*`, `/networks/*`, `/volumes/*` (v1 + versioned routes); state file `docker_api_containers.json`
- Diagnostics bundle (redacted) + scheduled-task installer (`05-Install-Api-Task.ps1`)
- Deployment scripts `scripts/windows/00..06` + `Invoke-StackDeckLocalDeploy.ps1`

## Enhanced / Improved Features Implemented (this session)

- **v8 r2 release hardening applied** — signed workflow, release scripts,
  runtime lock, source handover packaging, installed-client qualification, and
  docs now match the v8 remediation plan.
- **Machine-scoped release state** — production config defaults to
  `C:\ProgramData\StackDeck\config`; elevated scripts and Tauri first-run set
  `STACKDECK_CONFIG_DIR`; SMB secrets use LocalMachine DPAPI.
- **Production GUI surface reduced** — the legacy HTTP GUI command and
  production dependency graph edges are removed from CLI/hive builds.
- **Workspace now actually compiles & passes** — previously the `DEPLOYMENT_PHASES.md` honestly stated it had never been compiled/tested. Validated cold build + full test suite from a clean `target/`.
- **`cargo fmt --check` gate made green** — ran `cargo fmt` across the whole workspace (was failing on many files; purely cosmetic whitespace/line-wrap).
- **Fixed 7 broken `stackdeck-api` HTTP tests** — they returned `401` because the mandatory auth middleware requires `STACKDECK_DOCKER_API_TOKEN`, but those tests hit the router without a token/header. Added `TEST_TOKEN` const + `authed(builder)` helper; `isolated_home()` now seeds a default token; updated all 7 tests to send the Bearer header and use the serialized env lock.
- **Added `AGENTS.md`** — codifies the exact verification commands (fmt, scoped tests, release binaries), the `--workspace`-vs-scoped caveat, the API auth test gotcha, the deployment runbook + hard prerequisites, and the security posture. Future sessions auto-verify against these gates.

## Completed Work (chronological)

11. **2026-08-10** — Applied and extended the v8 r2 release-remediation patch. Source contract now enforces hidden release paths, per-machine NSIS, pinned runtime lock, no production legacy GUI dependency, ProgramData config root, LocalMachine DPAPI SMB blob, and generated-artifact hygiene. Validated PowerShell parser, workflow YAML, runtime lock JSON, LocalMachine DPAPI smoke, 104 locked scoped Rust tests, both locked release CLI/hive builds, CLI-only release script, desktop frontend build, and Tauri Rust check with temporary sidecar staging.
9. **2026-07-17** — Closed the remaining runtime blockers on this host. Switched the supported runtime image to Debian 12 genericcloud `debian-12-genericcloud-amd64.qcow2`, made the init/package flow default to that image, made the SHA optional for the one-command deploy path, installed `containernetworking-plugins` in cloud-init/bootstrap, and replaced unsupported `nerdctl --health-*` flags with an exec-based compose healthcheck probe. Verified the live Debian VM end to end: SSH reachable, `containerd` active, `nerdctl` working, compose smoke stack up with host port `127.0.0.1:18080`, and Docker API shim create/start/list/inspect against a real VM-backed container. Initialized a local git repo and committed the state as `df74c6f`.
10. **2026-07-16** — Host proof attempts after hardening still did **not** complete end-to-end. Isolated proof VM `stackdeck-proof-20260716` under `F:\StackDeck\VMs\stackdeck-proof-20260716` with isolated `USERPROFILE=F:\StackDeckProof\home`. Azure VHD archive booted and got IP but never opened SSH even with NoCloud+Azure OVF seed. Generic Ubuntu `.img` converted with qemu, booted and initially got IP but lost Hyper-V-reported IP / never reached SSH. KVM image booted but no IP was discoverable. OVA converted from VMDK, booted and got IP but never reached SSH. Therefore containerd/nerdctl bootstrap, compose workload proof, and real VM-backed Docker API proof remain unconfirmed.
7. **2026-07-15** — Ran current-host Hyper-V proof attempts. Admin/Hyper-V/OpenSSH/qemu prereqs passed; VM root selection to `F:\StackDeck\VMs\stackdeck-linux` worked; qemu VHDX conversion got past prior sparse/compressed VHDX `Resize-VHD` failure; Docker API shim loopback auth smoke passed (`/_ping`, `/version`, `/info`); repo validation passed (`cargo fmt --check`, scoped 66-test suite, release `stackdeck.exe` + `hive.exe` builds). Runtime bootstrap still failed before container workload proof because tested Linux images did not reach usable SSH readiness.
8. **2026-07-15** — Prior handoff reports hardening was committed and pushed as `c49528a Harden Hyper-V proof bootstrap`: real SSH probe failure detection; strict native-command exit checks in init/smoke/diagnostics scripts; qemu VHDX NTFS attribute normalization before `Resize-VHD`; removal of hardcoded `D:\` prereq assumption; MSI/API task ACL bug fix; explicit `openssh-server` install/start in cloud-init; automatic checkpoints disabled for runtime VMs; proof script support for qemu path, memory/CPU/disk options, and skipped build. This local workspace has no `.git`, so verify the pushed commit in the actual git checkout before relying on local history.
1. **2026-06-30** — Env scan: confirmed `cargo/rustc/node/npm/ssh/scp/tar/qemu-img` present, Hyper-V PowerShell module present (`Get-VM`), `D:` drive present, `admin=False`, `oscdimg` & `genisoimage` both missing.
2. **2026-06-30** — `cargo fmt --check` → failed → `cargo fmt` → recheck clean.
3. **2026-06-30** — Cold release build: `stackdeck.exe` in 1m38s, `hive.exe` in 51s; `stackdeck 0.5.0` runs; `doctor` reports `hyperv_cmdlets/ssh/scp: available`.
4. **2026-06-30** — `cargo test` found 7 failing `stackdeck-api` HTTP tests (all `401`). Diagnosed root cause (auth middleware + missing token). Fixed via `authed()`/`isolated_home()` token seeding. Re-ran: 66/66 pass.
5. **2026-06-30** — Re-verified: `cargo fmt --check` clean, release rebuild clean, `cargo check -p stackdeck-api` clean.
6. **2026-06-30** — Created `AGENTS.md` with verification gates + deployment context.

## Bugs Found and Fixes

| # | Bug | Root cause | Fix |
|---|-----|-----------|-----|
| 21 | Source handover ZIP could drop `.github`, `.gitignore`, or release scripts | Ad hoc filesystem/archive packaging did not make required hidden release paths a source contract | Added `13-Verify-Source-Release.ps1` and Git-manifest-driven `14-Package-Source-Handover.ps1` with post-ZIP required-path verification and manifest hashes |
| 20 | Release installer/state identity could split across per-user install/profile paths | NSIS defaulted to current-user install mode and config defaulted to the interactive user profile | Set NSIS `perMachine`; moved release config to `C:\ProgramData\StackDeck\config`; added private ACL initialization in elevated scripts and Tauri first-run |
| 19 | SMB credential storage did not match the machine-scoped release state model | CurrentUser DPAPI made elevated/setup/task identity sensitive | Switched SMB secret blob to LocalMachine DPAPI and kept ACLs as the access boundary |
| 18 | Legacy HTTP GUI remained a production attack surface | CLI still exposed/wired an HTTP GUI command and dependency path | Removed production `gui` command and `stackdeck-gui` workspace/dependency edges; source verifier fails if reintroduced |
| 17 | Runtime bootstrap inputs were mutable | Debian `/latest/` image and live apt/bootstrap inputs could change after qualification | Added populated `runtime/windows-runtime-lock.json`, Debian snapshot apt source, and nerdctl-full archive SHA-256 verification |
| 8 | `init_vm()` waited on stale in-memory SSH config after successful IP discovery | `create_cloud_vm()` saved `ssh_host` to disk but `init_vm()` kept using `self.config` | Reload config and bootstrap through a fresh `HyperVManager` |
| 9 | SSH probe falsely reported "reachable" for refused connections | `ssh_probe()` executed `ssh true` with `check=false`, so non-zero SSH exits still returned `Ok` | Run probe with `check=true` and a slightly longer process timeout |
| 10 | Default clean host could have QEMU installed but not on PATH | Chocolatey/QEMU standard install path is `C:\Program Files\qemu\qemu-img.exe`; backend only used `Get-Command` | Backend and prereq script now check standard QEMU install paths |
| 11 | `Get-FileHash` missing on this PowerShell environment | SHA verification assumed `Get-FileHash` existed | Added .NET SHA-256 fallback |
| 12 | Azure VHD and OVA seed formats were not represented | Seed was NoCloud-only ISO9660/Joliet | Seed now includes Azure `ovf-env.xml` and UDF as well; this did not make Azure/OVA proof pass on this host |
| 3 | qemu-converted VHDX hit sparse/compressed `Resize-VHD` failure | NTFS attributes on converted VHDX interfered with Hyper-V resize path | Prior pushed fix reports qemu VHDX NTFS attribute normalization before `Resize-VHD` |
| 4 | Bootstrap/diagnostic scripts could continue after failed native commands | PowerShell native command failures were not consistently promoted to terminating failures | Prior pushed fix reports strict native-command exit checks in init/smoke/diagnostics scripts |
| 5 | SSH readiness probe could mask real connection failures | Probe did not reliably detect failed SSH commands as failure states | Prior pushed fix reports real SSH probe failure detection |
| 6 | Runtime VM accumulated automatic checkpoints | Hyper-V default automatic checkpoints were not explicitly disabled for runtime VM | Prior pushed fix reports automatic checkpoints disabled |
| 7 | MSI/API scheduled task ACL issue | Installer/task ACL handling bug | Prior pushed fix reports MSI/API task ACL bug fix |
| 1 | `cargo fmt --check` fails | Source never formatted | `cargo fmt` (auto) |
| 2 | 7 `stackdeck-api` HTTP tests return `401` | `auth_middleware` requires `STACKDECK_DOCKER_API_TOKEN`; tests called `app()` without setting/sending a token. Production `serve()` is correct (refuses to start w/o token) — only tests were wrong | `TEST_TOKEN` const + `authed()` helper; `isolated_home()` seeds token; 7 tests send `Bearer` header. NOTE: `authed()` hardcodes `"Bearer test-token"` separately from `TEST_TOKEN` (minor drift risk — derive from const if touched) |

## Verification Notes

- **Verification commands** (run after any code change, from repo root):
  ```powershell
  cargo +1.88.0 fmt --check
  cargo +1.88.0 test -p stackdeck-api -p stackdeck-hyperv -p stackdeck-compose -p stackdeck-types -p stackdeck-state -p stackdeck-process --locked
  cargo +1.88.0 build -p stackdeck-cli --bin stackdeck-cli --release --locked
  cargo +1.88.0 build -p stackdeck-cli --bin stackdeck-hive --release --locked
  ```
- **Current test counts (2026-08-10):** API 26 · Hyper-V 25 · Compose 22 · Types 8 · State 8 · Process 15 = **104**. Older counts in dated historical entries describe earlier revisions.
- **Do NOT use `cargo test --workspace`** for routine checks — it pulls `desktop/stackdeck/src-tauri` (Tauri) which needs built frontend `dist/`, staged sidecar binaries, and WebView2. Use `cargo check -p stackdeck --locked` only after `npm run build` and temporary `npm run stage:hive`, then remove generated `desktop/stackdeck/src-tauri/bin/*.exe` before source handoff.
- **Binaries:** `target\release\stackdeck-cli.exe` and `target\release\stackdeck-hive.exe`. Both are `[[bin]]` targets over the same `main.rs`.
- **Non-admin limitation:** in a non-elevated shell, `Get-WindowsOptionalFeature -Online` fails and all VM/portproxy/SMB/scheduled-task operations are blocked. Hyper-V cmdlets still resolve; build/test are unaffected.
- **Release source contract:** run `scripts\windows\13-Verify-Source-Release.ps1` before claiming release readiness. It enforces `.github`, `.gitignore`, per-machine NSIS, pinned runtime lock, no production legacy GUI dependency, ProgramData config root, LocalMachine DPAPI, and generated-artifact hygiene.

## Known Regressions or Risks

### Historical deployment evidence (not proof of the current 0.5.1 install)

The entries in this subsection describe earlier VMs and revisions. The user later deleted the VHDX, and the 0.5.1 installation currently has no VHDX. Use the 2026-08-10 v8 r2 handoff and its unchecked external runtime gates as the current source of truth.

- **Current end-to-end proof (2026-07-17):** Debian genericcloud `debian-12-genericcloud-amd64.qcow2` now boots cleanly under Hyper-V on this host, reaches SSH, bootstraps `containerd` and `nerdctl`, starts the smoke compose stack, opens host port `127.0.0.1:18080`, and serves real containers through the Docker API shim.
- **Image-specific proof results (2026-07-17, isolated VM `stackdeck-proof-debian`):**
  - Debian 12 genericcloud qcow2: cloud-init seed consumed successfully, SSH reachable, `containerd` active, `nerdctl` functional, compose smoke stack started, host port 18080 reachable, Docker API create/start/list/inspect confirmed against a real VM-backed container.
- **Image-specific proof results (2026-07-16, isolated VM `stackdeck-proof-20260716`):**
  - Official Azure VHD archive `jammy-server-cloudimg-amd64-azure.vhd.tar.gz`: extracted/converted, VM booted and got IP, seed ISO contained NoCloud + Azure OVF, but SSH never opened within extended waits.
  - Official generic Ubuntu `.img`: qemu conversion succeeded; VM booted and initially got IP, then Hyper-V stopped reporting IP; SSH never opened.
  - Official KVM image `jammy-server-cloudimg-amd64-disk-kvm.img`: qemu conversion succeeded; VM started but IP discovery failed within 1200s.
  - Official OVA `jammy-server-cloudimg-amd64.ova`: extracted VMDK and converted; VM booted and got IP, but SSH never opened within 1200s.
  - Current Ubuntu SHA values used on 2026-07-16: generic `.img` `7cd6b514bcc6c43180270f7817efecf140297edd0768351fddfbed9858bd21b7`, KVM `380d4b3867773a7b9ff45ee6a5cae950b3581ed6a241b856a6ca2186d8edecec`, Azure archive `53710f4d514d75942087c25bc7065b008ca1bf4db03e09cf15bcc87b28117803`, OVA `84efa80e60f72ce5569f7c813dcf67c2cf78fc1d6033a35fae6b485972d58cfc`.
- **Image-specific proof results (2026-07-15):**
  - Official KVM qcow2: VM boots according to Hyper-V event log, but no usable IP/heartbeat path.
  - Local Azure VHDX: VM gets Hyper-V heartbeat/IP, but TCP/SSH on port 22 refuses for 900s.
  - Official generic Ubuntu `.img`: SSH readiness also fails within 900s.
- **Proof evidence:** `.stackdeck\current-host-proof-generic\proof.log`, `.stackdeck\current-host-proof-azure\proof.log`, `.stackdeck\current-host-proof\proof.log`.

- **SHA placeholder user error (resolved):** operator passed literal `<official-sha256>` to `-Sha256`, so `init` failed with SHA mismatch. **Correct, Ubuntu-verified SHA for `jammy/current/jammy-server-cloudimg-amd64.img` = `070de108b25df4c9eacc4a297c6afc583bdd58cabf159e61d4152bc6541a6b54`** (confirmed against official `https://cloud-images.ubuntu.com/jammy/current/SHA256SUMS`). Note: `jammy/current` is a rolling pointer — re-verify SHA if the image is re-downloaded on a later date.
- **oscdimg/genisoimage no longer a hard blocker:** `write_cloud_init_seed` falls back to Windows IMAPI2 when neither tool is present.
- **Leftover VM `stackdeck-linux`:** a prior attempt left a VM (IP `172.23.84.195`) with `ssh: unreachable`. `init` cannot recreate over it (throws "already exists"). Recovery options: (A) wait/retry `hyperv health` + `bootstrap` if SSH comes up; (B) `Remove-VM` + delete `D:\StackDeck\VMs\stackdeck-linux\stackdeck-linux.vhdx`, then re-init (needs the ISO tool).
- **Path A (salvage) ruled out, 2026-06-30:** agent probed `172.23.84.195` from a non-admin shell — TCP:22 CLOSED/FILTERED, ICMP 0 replies, ssh `Connection timed out`. The whole host is unreachable (VM stopped or Default Switch reassigned IP), not just a slow SSH boot. Bootstrap cannot connect. Also confirmed the agent's shell cannot run `Get-VM`/`Start-VM`/`Remove-VM`/`discover-ip` (need admin) — so both salvage and recreate paths require the **operator's elevated shell**. Stale IP in config is `172.23.84.195`; must re-run `discover-ip` elevated to get the current IP.
- **Historical SMB note superseded:** older entries described plaintext or per-user SMB credential handling. The current v8 r2 source stores SMB credentials outside JSON as a LocalMachine DPAPI blob under `C:\ProgramData\StackDeck\config`.

### Pre-existing risks

- **Phase 4 cloud-init ISO no longer requires `oscdimg`/`genisoimage`**; both are optional. The default Ubuntu `.img` path does require `qemu-img` unless a pre-converted VHDX is provided.
- **Phases 3–7 unrunnable without elevation** — VM creation, `netsh portproxy`, SMB shares, Highest-run-level task all need Administrator.
- **SMB credentials** are no longer serialized into `hyperv.json`; current release source stores them as a LocalMachine DPAPI blob under the private ProgramData config root. Any plaintext credential note below this line is historical only.
- **Docker API shim** must remain loopback-bound unless `--allow-remote` behind a hardened boundary.
- **`authed()` token duplication** — `"Bearer test-token"` literal is not derived from `TEST_TOKEN`; minor drift risk if the constant changes. Low severity (test-only).
- **Untracked build/test state**: no longer applies; repo is now a local git repository with commit `df74c6f`.
- **`qemu-img` on this host:** Chocolatey reports QEMU installed; executable is `C:\Program Files\qemu\qemu-img.exe` and is now found by the backend even when absent from `PATH`.

## Session History

- **2026-08-10 (v8 r2 GO-readiness remediation):** Inspected `StackDeck-HyperV-Local-Container-8-patched-scripts-v0.6.0-r2.zip`, applied the overlay, manually merged the signed-release workflow, and went beyond the patch with a machine-scoped state model. Production config now defaults to `C:\ProgramData\StackDeck\config`, SMB secrets use LocalMachine DPAPI, elevated scripts/Tauri setup export `STACKDECK_CONFIG_DIR`, NSIS is per-machine, the legacy HTTP GUI is removed from the production CLI/workspace graph, runtime inputs are pinned in `runtime/windows-runtime-lock.json`, source handovers are Git-manifest based, and installer verification compares exact pre-bundle payload hashes. Local gates passed: source verifier, PowerShell parser, YAML/JSON parse, DPAPI smoke, 104 locked scoped tests, locked release CLI/hive builds, CLI-only release script, desktop frontend build, and Tauri Rust check with temporary sidecar staging. Still pending: final signing, protected CI attestations, clean-machine installer qualification, and live Hyper-V runtime proof.

- **2026-08-01 (follow-up concurrency/runtime/signing gates):** Closed the second-review PID grace-window, OS-lock ownership, SQLite stale-merge, command-pipe, pull-buffering/line-framing, ephemeral-port reservation, API start crash-consistency, default loopback authentication, mirror-removal, and safe replacement findings. Delivered a corrected, commit-pinned signed-release workflow that signs desktop payloads before bundling and publishes hashes/provenance/attestations. Advanced the release to 0.5.2 because 0.5.1 is already installed. Rust 1.88 formatting, 103 scoped tests, both release CLI builds, Tauri production PE verification, fresh 0.5.2 MSI/NSIS generation, and MSI payload extraction passed. The fresh local installers remain unsigned, the installed Program Files build predates these follow-up fixes, and live Hyper-V initialization remains pending.

- **2026-08-01 (release remediation, first-run provisioning, and UI/package recovery):** Applied and refined the release-blocker overlay; added custom D-drive 8 GB dynamic-VHDX provisioning with no default SMB requirement; added the desktop first-run setup experience and installed-runtime initializer; corrected production Tauri packaging after an invalid dev-server-bound package; fixed UTF-8 BOM handling that blocked the VM view; built and locally installed StackDeck 0.5.1; and revalidated 98 scoped tests, locked release builds, production frontend embedding, and final MSI/NSIS hashes. The installed UI works, but the VHDX/VM was not created because the UAC-backed provisioning run did not complete. Public release remains blocked on live Hyper-V E2E, Authenticode signing, and clean-machine MSI/NSIS rehearsals.

- **2026-07-17 (runtime completion + one-click path):** Finished the production deploy path on this host. Verified that the Debian default image, CNI plugins, healthcheck probing, compose startup, and Docker API shim all work together. Converted the Windows deployment scripts to make `Sha256` optional and to default to the proven Debian qcow2 path. Initialized git and committed the tree locally as `df74c6f`.

- **2026-07-15 (current-host proof + hardening handoff):** Prior session reports commit `c49528a Harden Hyper-V proof bootstrap` was pushed to `main` in a GitHub checkout, but this local folder is currently not a git repository. The proof was honest but not end-to-end: prereqs, qemu conversion hardening, loopback authenticated API smoke, fmt/tests/release builds passed; VM bootstrap failed before container workload proof across official KVM qcow2, local Azure VHDX, and official generic Ubuntu `.img` due to missing usable IP/heartbeat or SSH readiness. Remaining blocker is first-boot Linux image provisioning/cloud-init/SSH readiness.

- **2026-06-30 (session 2, elevated operator run):** Operator ran Phase 1 (`-EnableFeatures`) — Hyper-V now `Enabled`, admin `OK`, all prereqs OK except `oscdimg`/`genisoimage`. Phase 4 `init` failed: passed the `<official-sha256>` placeholder → SHA mismatch (computed `070de108…`). Agent verified that hash against Ubuntu's official SHA256SUMS (exact match) and handed the operator the real value plus a two-track recovery (salvage-and-bootstrap vs. install-ISO-tool + remove-leftover-VM + re-init). Open: oscdimg/genisoimage install, leftover-VM cleanup, SMB configure.

- **2026-06-30 (session 1):** Validated Phase 2 (build+test) end-to-end and fixed the blockers; created `AGENTS.md` and `MEMORY.md`. Phases 1 (read-only) and 2 complete; Phases 3–7 handed to the operator to run from an elevated shell. Outstanding operator inputs: install `oscdimg`/`genisoimage`, provide Ubuntu image SHA-256 + Windows SMB password.

---

## Maintenance Instructions

**Update this file before context cleanup / handoff.** Specifically:

1. **Update [Quick Status](#quick-status-as-of-last-session)** table — flip states and update blockers.
2. **Append to [Completed Work](#completed-work-chronological)** — one dated bullet per work unit.
3. **Record any new bug** in [Bugs Found and Fixes](#bugs-found-and-fixes) with root cause + fix.
4. **Update [Verification Notes](#verification-notes)** if commands/counts change.
5. **Add to [Known Regressions or Risks](#known-regressions-or-risks)** anything that could bite the next session.
6. **Refresh [Session History](#session-history)** with a one-paragraph summary.
7. Keep entries **append-mostly**; newest first within a section. Do not delete historical entries — they are the audit trail.
8. If the repo gains git, note it here and switch change-tracking from manual to `git diff`.

**Handoff checklist:** [Quick Status] current · [Completed Work] dated · [Bugs] logged · [Risks] current · [Verification] reproducible · [Session History] summarized.
