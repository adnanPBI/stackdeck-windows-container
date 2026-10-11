# Validation record

## 2026-08-09 local audit-patch application

Applied the `StackDeck-HyperV-Local-Container-7-patched-scripts-v0.6.0-r1.zip`
overlay, then reconciled the release workflow/docs so they match the audit
findings and the current code. Additional local corrections include pinned
GitHub Actions in `.github/workflows/release-windows.yml`, tag-release asset
publication for the signed MSI/NSIS plus evidence, manifest self-verification,
certificate cleanup, removal of stale generated desktop/sidecar/state artifacts,
and documentation updates for scoped validation, DPAPI SMB credential storage,
default Debian digest resolution, and the Windows 11 Pro/Enterprise/Education
x86-64 support baseline.

Local validation performed on this Windows host:

- `cargo fmt --check`: passed after applying `cargo fmt`.
- Scoped core suite passed: API 26, Compose 22, Hyper-V 25, Process 15, State
  8, Types 8, plus doc-test crates.
- `cargo build -p stackdeck-cli --bin stackdeck-cli --release`: passed.
- `cargo build -p stackdeck-cli --bin stackdeck-hive --release`: passed after
  temporarily stopping and then restarting the local `StackDeck Local Docker
  API` scheduled task that had locked the old `target\release\stackdeck-hive.exe`.
- `scripts\windows\01-Build-Release.ps1 -SkipTests -SkipDesktop`: passed and
  printed the corrected `$StackDeckCliOutput` / `$StackDeckHiveOutput` paths.
- PowerShell parser over repository `.ps1` files: passed.
- `.github\workflows\release-windows.yml` YAML parse: passed.
- `git diff --check`: passed; Git emitted only line-ending normalization
  warnings.

This does not replace the remaining external release gates: protected
Authenticode signing with the real certificate, signed MSI/NSIS generation,
installer payload extraction on the signed artifacts, clean-machine installed
client qualification, and live Administrator-backed Hyper-V runtime
qualification still must be executed and archived before a client GO.

## 2026-08-09 release-safety patch status

The source patch now contains `.github/workflows/release-windows.yml` plus
`scripts/windows/09-Build-Signed-Release.ps1` and
`scripts/windows/10-Verify-Installer-Payloads.ps1`. The canonical release path
is `target/x86_64-pc-windows-msvc/release`; the workflow signs desktop/sidecar
payloads before bundling, signs MSI/NSIS afterward, extracts both installers
with 7-Zip to verify embedded Authenticode signatures, writes SHA-256 and JSON
provenance, and creates a pinned GitHub `actions/attest` v4 build-provenance
attestation. This source patch does **not** claim that a real certificate or
live Hyper-V qualification was executed; those remain external evidence gates.

First-run readiness is now tied to the actual VM disk plus a strict VM/SSH/
containerd/nerdctl/BuildKit health gate. Desktop child-process output is bounded
and timed-out processes have a finite termination grace period. SMB passwords
are excluded from JSON and stored as LocalMachine DPAPI-protected data under
the machine-scoped StackDeck config root.

## StackDeck namespace migration validation (2026-08-01)

- Completed the breaking product-namespace migration to **StackDeck 0.6.0**.
  Crate/package names, Rust imports, CLI binaries, environment variables,
  state/registry paths, Hyper-V labels and defaults, API task/data names,
  scripts, workflow references, tests, and documentation now use StackDeck.
- The non-colliding Windows executables are `stackdeck-cli.exe`,
  `stackdeck-hive.exe`, and the Tauri desktop `stackdeck.exe`.
- A case-insensitive source/path scan excluding generated caches found zero
  occurrences of the retired namespace. The generated Cargo cache was cleaned;
  only the five canonical 0.6.0 release artifacts were restored.
- Rust 1.88 formatting passed. The scoped core suite passed **103 tests** plus
  doc tests: API 26, Compose 22, Hyper-V 25, Process 15, State 8, Types 7.
- Release builds passed for `stackdeck-cli` and `stackdeck-hive`; both report
  version 0.6.0. Tauri production build, frontend embedding guard, sidecar
  version guard, and MSI/NSIS bundling passed.
- MSI administrative extraction found only `stackdeck.exe` and
  `stackdeck-hive.exe` executable payloads and no retired namespace.
- The live scheduled task and token directory were migrated in place to
  `StackDeck Local Docker API` and `C:\ProgramData\StackDeck`. An authenticated
  request to `http://127.0.0.1:23750/_ping` returned HTTP 200 `OK`.

| Unsigned local artifact | Bytes | SHA-256 |
|---|---:|---|
| `target\release\bundle\msi\StackDeck_0.6.0_x64_en-US.msi` | 214,650,880 | `4F526E21A22B5687C59E6AA17460138E3E08FED2958E5DFFFB105680B6B268BA` |
| `target\release\bundle\nsis\StackDeck_0.6.0_x64-setup.exe` | 215,506,726 | `915866A7DF1AF2D9392E05507043CA3E956C4195D524F041C9AE5A7A1B997BBD` |

Both packages are `NotSigned` local test artifacts and must not be published.
The installed Program Files desktop remains 0.5.1; the running 0.6.0 API task
currently points to the repository release sidecar until a signed 0.6.0 package
is installed. Live VHDX/Hyper-V initialization and clean-machine installer
qualification remain pending.

## Source validation performed on Windows (2026-07-30)

- The original overlay targets baseline `d0ba6728f8107653e3a07d833a0e12e20efd958d`;
  every archive member matched `PATCH-CONTENTS.sha256` before application.
- `cargo +stable fmt --check`: passed.
- `cargo +stable test -p stackdeck-api -p stackdeck-hyperv -p stackdeck-compose
  -p stackdeck-types -p stackdeck-state -p stackdeck-process --locked`: passed
  (97 unit tests plus doc tests).
- `cargo +stable build -p stackdeck-cli --bin stackdeck-cli --release --locked`:
  passed; produced `target\\release\\stackdeck-cli.exe`.
- `cargo +stable build -p stackdeck-cli --bin stackdeck-hive --release --locked`: passed;
  produced `target\\release\\stackdeck-hive.exe`.
- Every `scripts\\windows\\*.ps1` file parsed successfully with the PowerShell
  parser.
- `npm.cmd run build` in `desktop\\stackdeck`: passed.
- `git diff --check`: passed.

The source checks used installed Rust stable 1.94 because the repository-pinned
Rust 1.88 toolchain was not installed locally. CI and release builds must still
run the pinned 1.88 MSVC toolchain.

## Environment-only release gates still required

- Administrator Hyper-V lifecycle, SSH bootstrap, DHCP recovery, BuildKit,
  SMB mounts, portproxy/firewall cleanup, and authenticated live-registry pull.
- Clean-machine WebView2 installation plus MSI and NSIS launch.
- Authenticode signing/verification and provenance attestation using the real
  release certificate.

These are deployment evidence gates, not remaining source defects. The signed
release workflow and `PATCH-README.md` provide the required commands.

## Desktop runtime-setup release validation (2026-08-01)

- Remediation release version bumped to `0.5.1` so Windows Installer upgrades
  the earlier `0.5.0` package instead of treating it as the same product build.
- `cargo +1.88.0 fmt --check`: passed using the pinned release toolchain.
- Scoped core tests: passed (98 unit tests plus doc tests).
- `stackdeck-cli.exe`, `stackdeck-hive.exe`, and the Tauri desktop executable: release builds
  passed with Rust 1.88; the staged and installed hive sidecars report `0.5.1`.
- `npm.cmd run build`: passed.
- PowerShell parser: all 12 deployment and desktop packaging scripts passed.
- `git diff --check`: passed.
- MSI administrative extraction verified that its desktop executable and hive
  sidecar exactly match the current release-build SHA-256 values.
- MSI `0.5.1` installation into `C:\Program Files\StackDeck`: exit 0; installed
  desktop and sidecar hashes exactly match the current build; registered
  `DisplayVersion` is `0.5.1`.
- MSI SHA-256:
  `52CA02210ECE5945CB561ADA77F7DD5A31FD18FE66763DFB42C729F16D35095C`.
- NSIS SHA-256:
  `1EDF6918A9D57EB2E09823DC63D0CF4AD0B9AEC79FF85124907131BAD1F7DD80`.

An initial `0.5.1` packaging attempt used `cargo build -p stackdeck` followed by
`tauri bundle`; that binary opened the configured development URL
`127.0.0.1:1420` and was rejected. Both invalid installers were moved to files
whose names contain `dev-server-broken`. The production package workflow now
runs `tauri build --no-bundle --ci` and `verify-release-binary.ps1` before
bundling. MSI extraction confirmed that the replacement executable embeds the
current hashed Vite asset; its installed SHA-256 exactly
matches the corrected release binary.

The installed production UI was visually checked after replacement: the main
dashboard rendered without a localhost connection and the VM tab rendered the
custom-location setup form with
`D:\StackDeck\VMs\stackdeck-linux`, 8 GB, and the create button. A legacy
PowerShell-written `hyperv.json` then exposed a UTF-8 BOM incompatibility. The
loader now strips only an optional BOM before strict JSON parsing, a regression
test covers the legacy file, and the UI shows a retryable error instead of an
indefinite loading panel. The installed sidecar normalized the legacy file and
`hyperv doctor` exited 0.

Both installers currently report `NotSigned`. They are valid local test
artifacts, but must pass `scripts\windows\07-Sign-Release.ps1` with the real
release certificate before public GitHub/Medium distribution.

The live Hyper-V initialization gate remains pending at the Windows integrity
boundary: an Administrator must approve the desktop UAC prompt or run
`scripts\windows\08-Initialize-Installed-Runtime.ps1`. No VHDX existed under
`D:\StackDeck\VMs\stackdeck-linux` at the end of this validation pass.

## Follow-up release-blocker validation (2026-08-01)

The second review identified narrower race windows and a release-workflow gap.
The worktree now includes the following additional remediations:

- PID identity is re-read after the graceful-stop delay and immediately before
  force termination.
- JSON state locks use stable OS file locks; lock paths are neither age-deleted
  nor unlinked by a guard destructor.
- SQLite service upsert is one atomic `INSERT ... ON CONFLICT` merge. A
  concurrent disjoint-field regression test uses two independent connections.
- Captured PowerShell/SSH pipes are drained while children run; captured tails
  and pull line buffers are bounded.
- Pull streaming uses a bounded synchronous pump before the bounded Tokio
  channel, separate stdout/stderr line decoders, and client-disconnect
  cancellation.
- Docker create retains bound host listeners and adds each allocation to the
  same request reservation set. Reservations survive until guest start and are
  released immediately before portproxy activation.
- API start commits `runtime_pending=true` before guest mutation and resolves
  the deterministic runtime name during cleanup, so `runtime_id: null` cannot
  silently hide a possible live container.
- Docker API bearer authentication is required by default on loopback. The
  unauthenticated compatibility mode is now an explicit opt-out.
- Mirror removal deletes the guest `hosts.toml` directory and restarts
  containerd before local JSON state is committed.
- Containers carry a configuration-hash label. Drift and explicit builds use a
  stop/rename/launch/healthcheck/rollback replacement sequence rather than
  deleting the old container first.

Validation after these changes:

- `cargo +1.88.0 fmt --check`: passed.
- Scoped core suite: **103 unit tests passed**, plus doc tests: API 26,
  Compose 22, Hyper-V 25, Process 15, State 8, Types 7.
- Locked Rust 1.88 release builds for both `stackdeck-cli.exe` and `stackdeck-hive.exe`: passed.
- Tauri `build:desktop-release`: passed; the production PE contains the current
  hashed Vite asset and runtime setup commands, and the staged hive reports
  `0.5.1`.
- MSI administrative extraction: passed; embedded desktop and hive hashes
  match the current production PE and staged sidecar.
- PowerShell parser for `07-Sign-Release.ps1`: passed.
- Historical GitHub workflow parsing passed for the earlier local remediation.
  The current release-safety patch replaces that workflow; the authoritative
  current contract is the 2026-08-09 section at the top of this file.
- `git diff --check`: passed with line-ending warnings only.

Because 0.5.1 was already installed on this host, the follow-up remediation was
versioned as **0.5.2** so Windows Installer upgrade rules cannot retain the
older binaries. Fresh locally generated, **unsigned** test installers:

| Artifact | Bytes | SHA-256 | Authenticode |
|---|---:|---|---|
| `target\release\bundle\msi\StackDeck_0.5.2_x64_en-US.msi` | 214,646,784 | `81C6E003A8CB083314B1CF0D5A67A7C2C901A2D4E17943AEE23151C34E4CAC98` | `NotSigned` |
| `target\release\bundle\nsis\StackDeck_0.5.2_x64-setup.exe` | 215,502,408 | `C553D7B9B1053D07C0592A45A44D8AB64DE9983F7FA5B7DCDE447214FAB63007` | `NotSigned` |

The final 0.5.2 `tauri bundle --bundles msi,nsis --ci` command exited 0 after
WiX and NSIS produced both canonical artifacts. The MSI payload was
independently extracted and hash-matched to the current production desktop PE
and staged 0.5.2 hive sidecar. This is artifact evidence, not a successful
Authenticode release run.

The signed workflow is delivered in `.github/workflows/release-windows.yml`.
It builds the production payloads before bundling, signs the desktop executable
and staged sidecar first, creates MSI/NSIS, signs both installers, extracts both
installers to verify embedded executable signatures, writes `SHA256SUMS.txt`
and `provenance.json`, and requests a GitHub build-provenance attestation. The
workflow has not been executed in this source-only audit because the protected
PFX/password secrets and trusted release certificate are not available here.

Release remains blocked on two external evidence gates:

1. Run the protected GitHub `Signed Windows release` workflow with the real
   certificate and verify the published signatures, timestamp chains, hashes,
   provenance, and attestation.
2. Complete Administrator/UAC-backed Hyper-V initialization and the live VM,
   SSH, BuildKit, Compose, API, restart/DHCP, portproxy cleanup, registry mirror
   removal, and replacement/rollback scenarios. No current VHDX means these
   runtime claims remain unproven.

## v8 non-signing remediation contract (2026-08-10)

The source patch following the v8 re-audit adds/changes the release contract as follows:

- restores `.github/workflows/release-windows.yml` and `.gitignore`;
- makes NSIS installation explicitly `perMachine`;
- replaces the D:-specific installed-runtime default with `%SystemDrive%`;
- moves release config/keys/tokens/SMB blobs to `C:\ProgramData\StackDeck\config`;
- rejects desktop first-run when elevation would require a different administrator identity;
- removes the legacy HTTP `gui` command from the production CLI/hive command surface;
- requires a pinned runtime lock instead of resolving Debian `bookworm/latest` during normal client installation;
- pins Debian apt bootstrap to a release-qualified snapshot and nerdctl to a qualified archive SHA-256;
- records canonical signed payload hashes before bundling and requires exact hash identity after MSI/NSIS extraction;
- streams the SecureString SMB password to the child CLI through stdin;
- adds an explicit `-SkipSignature` mode to installed-client qualification for pre-sign functional testing only;
- adds source-handover packaging that preserves and post-verifies dotfiles/hidden release paths.

`runtime/windows-runtime-lock.json` is populated for this source tree with a dated Debian Bookworm cloud image URL, SHA-512, Debian snapshot timestamp, nerdctl version, and nerdctl amd64 SHA-256. `01-Build-Release.ps1`, `09-Build-Signed-Release.ps1`, and `14-Package-Source-Handover.ps1` fail closed if those values are removed or made mutable.

The following remain external evidence gates rather than code changes: clean-machine MSI/NSIS install/repair/upgrade/uninstall qualification and Administrator-backed live Hyper-V/SSH/containerd/nerdctl/BuildKit/Compose/restart/cleanup/rollback qualification on every supported Windows client class. Final Authenticode signing/provenance remains a separate final release gate.

## v8 r2 local validation evidence (2026-08-10)

Local source/build gates run after applying the r2 remediation overlay and the
machine-scoped config/DPAPI hardening:

- `scripts\windows\13-Verify-Source-Release.ps1`: PASS.
- PowerShell parser over `scripts\windows\*.ps1`: PASS.
- `.github\workflows\release-windows.yml` YAML parse: PASS.
- `desktop\stackdeck\src-tauri\tauri.conf.json` JSON parse: PASS.
- LocalMachine DPAPI protect/unprotect smoke test with `ProtectedData`: PASS.
- `cargo fmt --check`: PASS.
- Scoped Rust tests with `--locked`: PASS, 104 tests across `stackdeck-api`,
  `stackdeck-hyperv`, `stackdeck-compose`, `stackdeck-types`,
  `stackdeck-state`, and `stackdeck-process`.
- `cargo build -p stackdeck-cli --bin stackdeck-cli --release --locked`: PASS.
- `cargo build -p stackdeck-cli --bin stackdeck-hive --release --locked`: PASS.
- `scripts\windows\01-Build-Release.ps1 -SkipTests -SkipDesktop`: PASS.
- `npm run build` in `desktop\stackdeck`: PASS.
- `cargo check -p stackdeck --locked`: PASS after temporary sidecar staging;
  generated `src-tauri\bin\*.exe` files were removed afterward and the source
  verifier passed again.

The runtime lock is populated with:

- Debian image:
  `https://cloud.debian.org/images/cloud/bookworm/20260806-2562/debian-12-genericcloud-amd64-20260806-2562.qcow2`
- Debian image SHA-512:
  `3622c990108a044ed411652f8741e77c5822c365114d7b940206b243f8fb617b8586792df4cdb7afba1b71d1a09289d8ed632124688f2c8352cb08190a1e9868`
- Debian snapshot timestamp: `20260806T120000Z`.
- nerdctl full archive: `nerdctl-full-2.3.5-linux-amd64.tar.gz`.
- nerdctl full archive SHA-256:
  `b697295c623639734aaab737523c808fd3cc8d3046039fd94fff1744e4c317aa`.

Still not claimed from this local source validation: final Authenticode-signed
MSI/NSIS production artifacts, GitHub artifact attestations from protected CI,
and clean-machine live Hyper-V runtime qualification.
