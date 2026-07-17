# MEMORY.md — Running Journal

> **Living document.** This is the agent's durable memory across sessions. Append (do not rewrite history) at the end of every work session, before context cleanup/handoff. Newest entries on top within each section. See [Maintenance Instructions](#maintenance-instructions) at the bottom.

**Repo root:** `D:\Coding Development Files\PyStack-HyperV-local-deployment-ready`
**Not a git repo** (no `.git`) — there is no diff baseline; track changes manually or via the timestamps here.
**Target:** Windows host, Hyper-V-backed, WSL-free / Docker-Desktop-free container runtime with Docker-compatible CLI/API.

---

## Quick Status (as of last session)

| Item | State |
|------|-------|
| Source compiles | ✅ release CLI builds clean |
| `cargo fmt --check` | ✅ clean |
| Unit tests | ✅ 69/69 pass |
| `pystack.exe` / `hive.exe` | ✅ built (`target\release\`) |
| Phase 1 prereqs (elevated) | ✅ Admin/Hyper-V/OpenSSH/qemu prereqs pass |
| Docker API shim | ✅ loopback API starts; authenticated `/_ping`, `/version`, `/info` pass |
| Phase 4/init runtime bootstrap | ❌ still blocked on this host — Canonical images boot inconsistently under Hyper-V and do not reach stable SSH |
| End-to-end runtime proof | ❌ not complete — containerd/nerdctl bootstrap, compose workload, and real VM-backed API proof did not run |
| Current checkout note | ⚠️ This workspace is not a git repository; prior handoff says commit `c49528a` was pushed elsewhere |
| Blockers to end-to-end run | Default/generic Ubuntu `.img`, OVA, KVM image, and Azure VHD paths were all tested after hardening; none reached container bootstrap; see Deployment Blockers below |

---

## Project Summary

Rust workspace delivering a local Windows container runtime that is deliberately independent of WSL, Docker Desktop, and any host Docker Engine. The CLI/API drive a Hyper-V Linux VM that runs `containerd + nerdctl` to host OCI containers, with `netsh portproxy` + SMB bind mounts bridging host and VM.

Architecture chain: `Windows host → Rust CLI/Tauri desktop → Hyper-V Linux VM → containerd + nerdctl → OCI containers`.

## Current Architecture

**Workspace members (`Cargo.toml`, edition 2021, rust 1.75):**
- `crates/pystack-types` — shared config/types (`HyperVConfig`, `HyperVService`, registry paths, redaction hints)
- `crates/pystack-compose` — docker-compose.yml parsing → `ComposeProject`
- `crates/pystack-state` — state store
- `crates/pystack-process` — native process backend (sqlite via `rusqlite` bundled)
- `crates/pystack-hyperv` — Hyper-V backend: VM lifecycle (PowerShell), SSH/nerdctl orchestration, cloud-init seed ISO, qcow2→VHDX, portproxy, SMB shares, registry mirrors
- `crates/pystack-api` — Docker Engine API shim (axum 0.8), bearer-token auth middleware, loopback-bound
- `crates/pystack-gui` — legacy/offline GUI (eframe/axum)
- `crates/pystack-cli` — `pystack` + `hive` bins (both `[[bin]]` → `src/main.rs`)
- `desktop/stackdeck/src-tauri` — Tauri 2 + React desktop UI (needs Node `dist/` + WebView2 to build/package)

**Release profile:** `strip=true`, `opt-level="z"`, `lto=true`, `codegen-units=1` (small binary, slower link).

**Config layout:** Hyper-V config persisted to a registry dir (under `%USERPROFILE%`/`USERPROFILE`); SMB password stored **plaintext** by design (local-only, documented in `SECURITY_REVIEW.md`).

## Implemented Features (existing, pre-session)

- CLI: `init/validate/doctor/up/down/restart/status/logs/diagnostics/watch/register/unregister/gui`
- Compose: `up/build/down/status/logs` with dependency-condition waiting (`service_started`/`service_healthy`/`service_completed_successfully`), supervise mode
- Hyper-V: `configure/ensure-key/doctor/init/download-image/create-cloud-vm/start/stop/ip/discover-ip/bootstrap/health/repair/ps/exec`, plus `snapshot/image/volume/network/share/mirror` subcommands
- Docker API shim: `/_ping`, `/version`, `/containers/*`, `/images/*`, `/networks/*`, `/volumes/*` (v1 + versioned routes); state file `docker_api_containers.json`
- Diagnostics bundle (redacted) + scheduled-task installer (`05-Install-Api-Task.ps1`)
- Deployment scripts `scripts/windows/00..06` + `Invoke-PyStackLocalDeploy.ps1`

## Enhanced / Improved Features Implemented (this session)

- **Workspace now actually compiles & passes** — previously the `DEPLOYMENT_PHASES.md` honestly stated it had never been compiled/tested. Validated cold build + full test suite from a clean `target/`.
- **`cargo fmt --check` gate made green** — ran `cargo fmt` across the whole workspace (was failing on many files; purely cosmetic whitespace/line-wrap).
- **Fixed 7 broken `pystack-api` HTTP tests** — they returned `401` because the mandatory auth middleware requires `PYSTACK_DOCKER_API_TOKEN`, but those tests hit the router without a token/header. Added `TEST_TOKEN` const + `authed(builder)` helper; `isolated_home()` now seeds a default token; updated all 7 tests to send the Bearer header and use the serialized env lock.
- **Added `AGENTS.md`** — codifies the exact verification commands (fmt, scoped tests, release binaries), the `--workspace`-vs-scoped caveat, the API auth test gotcha, the deployment runbook + hard prerequisites, and the security posture. Future sessions auto-verify against these gates.

## Completed Work (chronological)

9. **2026-07-16** — Added more Hyper-V first-boot hardening: `init_vm()` now reloads the saved config after IP discovery before waiting for SSH/bootstrap; `repair()` uses a refreshed manager after discovering/saving IP; `wait_for_ssh()` refreshes stale IPs, ignores APIPA addresses, and requires consecutive successful probes; `ssh_probe()` now checks the SSH exit status instead of treating any completed `ssh` process as success; bootstrap retries transient SSH session failures; cloud-init seed includes `openssh-server`, explicit SSH start, Azure `ovf-env.xml`, and ISO9660+Joliet+UDF seed generation; qemu lookup now finds `C:\Program Files\qemu\qemu-img.exe`; VHD/VHDX/qemu conversion paths normalize sparse/compressed attributes; `.vhd.tar.gz` and `.ova` archive handling added. Added regression tests for cloud-init SSH, Azure OVF, archive handling.
10. **2026-07-16** — Host proof attempts after hardening still did **not** complete end-to-end. Isolated proof VM `pystack-proof-20260716` under `F:\StackDeck\VMs\pystack-proof-20260716` with isolated `USERPROFILE=F:\StackDeckProof\home`. Azure VHD archive booted and got IP but never opened SSH even with NoCloud+Azure OVF seed. Generic Ubuntu `.img` converted with qemu, booted and initially got IP but lost Hyper-V-reported IP / never reached SSH. KVM image booted but no IP was discoverable. OVA converted from VMDK, booted and got IP but never reached SSH. Therefore containerd/nerdctl bootstrap, compose workload proof, and real VM-backed Docker API proof remain unconfirmed.
7. **2026-07-15** — Ran current-host Hyper-V proof attempts. Admin/Hyper-V/OpenSSH/qemu prereqs passed; VM root selection to `F:\StackDeck\VMs\stackdeck-linux` worked; qemu VHDX conversion got past prior sparse/compressed VHDX `Resize-VHD` failure; Docker API shim loopback auth smoke passed (`/_ping`, `/version`, `/info`); repo validation passed (`cargo fmt --check`, scoped 66-test suite, release `pystack.exe` + `hive.exe` builds). Runtime bootstrap still failed before container workload proof because tested Linux images did not reach usable SSH readiness.
8. **2026-07-15** — Prior handoff reports hardening was committed and pushed as `c49528a Harden Hyper-V proof bootstrap`: real SSH probe failure detection; strict native-command exit checks in init/smoke/diagnostics scripts; qemu VHDX NTFS attribute normalization before `Resize-VHD`; removal of hardcoded `D:\` prereq assumption; MSI/API task ACL bug fix; explicit `openssh-server` install/start in cloud-init; automatic checkpoints disabled for runtime VMs; proof script support for qemu path, memory/CPU/disk options, and skipped build. This local workspace has no `.git`, so verify the pushed commit in the actual git checkout before relying on local history.
1. **2026-06-30** — Env scan: confirmed `cargo/rustc/node/npm/ssh/scp/tar/qemu-img` present, Hyper-V PowerShell module present (`Get-VM`), `D:` drive present, `admin=False`, `oscdimg` & `genisoimage` both missing.
2. **2026-06-30** — `cargo fmt --check` → failed → `cargo fmt` → recheck clean.
3. **2026-06-30** — Cold release build: `pystack.exe` in 1m38s, `hive.exe` in 51s; `pystack 0.5.0` runs; `doctor` reports `hyperv_cmdlets/ssh/scp: available`.
4. **2026-06-30** — `cargo test` found 7 failing `pystack-api` HTTP tests (all `401`). Diagnosed root cause (auth middleware + missing token). Fixed via `authed()`/`isolated_home()` token seeding. Re-ran: 66/66 pass.
5. **2026-06-30** — Re-verified: `cargo fmt --check` clean, release rebuild clean, `cargo check -p pystack-api` clean.
6. **2026-06-30** — Created `AGENTS.md` with verification gates + deployment context.

## Bugs Found and Fixes

| # | Bug | Root cause | Fix |
|---|-----|-----------|-----|
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
| 2 | 7 `pystack-api` HTTP tests return `401` | `auth_middleware` requires `PYSTACK_DOCKER_API_TOKEN`; tests called `app()` without setting/sending a token. Production `serve()` is correct (refuses to start w/o token) — only tests were wrong | `TEST_TOKEN` const + `authed()` helper; `isolated_home()` seeds token; 7 tests send `Bearer` header. NOTE: `authed()` hardcodes `"Bearer test-token"` separately from `TEST_TOKEN` (minor drift risk — derive from const if touched) |

## Verification Notes

- **Verification commands** (run after any code change, from repo root):
  ```powershell
  cargo fmt --check
  cargo test -p pystack-api -p pystack-hyperv -p pystack-compose -p pystack-types -p pystack-state -p pystack-process
  cargo build -p pystack-cli --bin pystack --release
  cargo build -p pystack-cli --bin hive --release
  ```
- **Test counts:** api 14 · hyperv 17 · compose 20 · types 6 · state 7 · process 5 = **69**.
- **Do NOT use `cargo test --workspace`** for routine checks — it pulls `desktop/stackdeck/src-tauri` (Tauri) which needs built frontend `dist/` + WebView2. `--workspace` is only for full desktop packaging (matches `scripts/windows/01-Build-Release.ps1`).
- **Binaries:** `target\release\pystack.exe` (~5.1 MB), `target\release\hive.exe`. Both `pystack` and `hive` are `[[bin]]` over the same `main.rs` (expected "found in multiple build targets" warning — by design).
- **Non-admin limitation:** in a non-elevated shell, `Get-WindowsOptionalFeature -Online` fails and all VM/portproxy/SMB/scheduled-task operations are blocked. Hyper-V cmdlets still resolve; build/test are unaffected.

## Known Regressions or Risks

### Deployment Blockers (live, 2026-06-30 elevated run)

- **Current end-to-end blocker (2026-07-16 proof):** first-boot Linux image provisioning/cloud-init/SSH readiness remains unresolved on this host despite code hardening. The runtime never reached `containerd`/`nerdctl` bootstrap, compose workload execution, or real VM-backed Docker API proof.
- **Image-specific proof results (2026-07-16, isolated VM `pystack-proof-20260716`):**
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
- **`02-Configure-HyperV.ps1` not yet run** in the elevated session — SMB creds (`windows_host`/`smb_user`/`smb_password`) may still be unset, which will block any bind-mount services later.

### Pre-existing risks

- **Phase 4 cloud-init ISO no longer requires `oscdimg`/`genisoimage`**; both are optional. The default Ubuntu `.img` path does require `qemu-img` unless a pre-converted VHDX is provided.
- **Phases 3–7 unrunnable without elevation** — VM creation, `netsh portproxy`, SMB shares, Highest-run-level task all need Administrator.
- **Plaintext SMB credentials** in `stack.json` — intentional local-only contract; do not add DPAPI/Credential Manager without coordinating (breaks documented posture).
- **Docker API shim** must remain loopback-bound unless `--allow-remote` behind a hardened boundary.
- **`authed()` token duplication** — `"Bearer test-token"` literal is not derived from `TEST_TOKEN`; minor drift risk if the constant changes. Low severity (test-only).
- **Untracked build/test state**: repo is not git; changes are only tracked here + file mtimes. Consider `git init` if you want real diff/revert.
- **`qemu-img` on this host:** Chocolatey reports QEMU installed; executable is `C:\Program Files\qemu\qemu-img.exe` and is now found by the backend even when absent from `PATH`.

## Session History

- **2026-07-16 (local hardening + repeated proof attempts):** Implemented the missing hardening in this workspace and added additional image handling. Validation passed: `cargo fmt --check`, scoped core tests 69/69, release `pystack.exe`, release `hive.exe`. Runtime proof still did not pass on this host after testing Azure VHD archive, generic `.img`, KVM image, and OVA. Current blocker is no longer false SSH detection; it is that Canonical artifacts under this host's Hyper-V setup do not reach stable SSH/guest readiness after conversion/seed.

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
