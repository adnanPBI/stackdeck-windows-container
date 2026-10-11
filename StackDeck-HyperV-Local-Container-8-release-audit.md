# StackDeck Hyper-V Local Container (8) — Deep Client GO Re-Audit

**Audit date:** 2026-08-09
**Artifact audited:** `StackDeck-HyperV-Local-Container(8).zip`
**Artifact SHA-256:** `77e4d3930414ca5c45f7009b4c00c544e553e7db75c7b893845a428e641f1fcf`

## Final verdict

**NO-GO as the exact client release candidate, even if MSI/NSIS Authenticode signing is explicitly excluded from the decision.**

The amended source is materially better than `(7)`. The important PID-reuse, OS file-lock, SQLite upsert, Hyper-V readiness, strict-health, process-timeout, image-digest, release-order, and DPAPI credential fixes are present.

However, independent non-signing release blockers remain:

1. The exact ZIP omits the Windows release workflow and `.gitignore` that its own validation record says are delivered.
2. The NSIS install location contract is inconsistent with the installed-runtime and qualification scripts.
3. Clean-machine installed-client qualification and live Hyper-V runtime qualification are still explicitly pending in `VALIDATION.md`.
4. The exact ZIP is source-only and contains no MSI/NSIS client installer to install or qualify.
5. The installed sidecar still contains a legacy HTTP management GUI that can bind to a non-loopback address despite the security documentation saying remote GUI access is unsupported by design.
6. Installed-runtime recovery defaults assume `D:` while the normal desktop path can use `C:`/the user profile.
7. Standard-user UAC with a different administrator identity can split StackDeck configuration/DPAPI state between two Windows user profiles.
8. The default runtime image/bootstrap is integrity-checked but mutable (`bookworm/latest` plus live apt/GitHub dependencies), so the runtime installed later is not the immutable runtime that was qualified with the application release.
9. Installer payload verification validates Authenticode signatures but does not hash-match extracted payloads to the exact canonical signed desktop/sidecar binaries that were bundled.

These findings prevent a defensible claim that the package is ready for arbitrary Windows 10/11 client installation.

---

# What is successfully remediated from the `(7)` audit

## PID-reuse forced termination safety — REMEDIATED

The process manager retains the earlier correction that revalidates process identity immediately before Windows forced termination after the graceful delay.

This closes the earlier PID-reuse window where an unrelated process could inherit the PID and be killed.

## Cross-process lock race — REMEDIATED

The state/types layer uses a stable OS-backed file lock instead of age-based lock-file deletion and unconditional unlink-on-drop.

The earlier lock deletion race is not present in the current source.

## SQLite read/modify/write race — REMEDIATED

State updates use a single SQLite `INSERT ... ON CONFLICT ... DO UPDATE` flow with concurrency-oriented tests rather than a separate read followed by write.

## Release build variable defect — REMEDIATED

`scripts/windows/01-Build-Release.ps1` now consistently uses:

- `$StackDeckCliOutput`
- `$StackDeckHiveOutput`
- `$StackDeckDesktopOutput`

The undefined `$StackDeckOutput` / `$HiveOutput` failure from `(7)` is gone.

## Canonical release paths — REMEDIATED in scripts

The build/sign scripts now consistently use:

- compatibility CLI/hive: `target\release`
- Tauri release: `target\x86_64-pc-windows-msvc\release`
- bundles: `target\x86_64-pc-windows-msvc\release\bundle`

## Signing/bundling order — REMEDIATED in local scripts

`scripts/windows/09-Build-Signed-Release.ps1` implements the correct sequence:

1. build/test;
2. sign CLI/hive;
3. sign desktop/staged sidecars;
4. bundle MSI/NSIS;
5. sign installers;
6. extract installers and verify embedded Authenticode;
7. write SHA-256 manifest/provenance.

That is materially safer than the earlier bundle-before-payload-sign flow.

## False-ready cached VHDX — REMEDIATED

`desktop/stackdeck/src-tauri/src/main.rs` now finds the actual per-VM disk:

`<vm_root>\<vm_name>.vhdx`

rather than accepting any cached base-image VHDX under `images`.

`runtime_setup_status()` also requires `HyperVManager::assert_runtime_ready()` to pass before reporting `ready=true`.

## Hyper-V health finality — REMEDIATED

`hyperv health` is now a strict automation/release gate:

- VM must be running;
- SSH must be reachable;
- containerd must be active;
- nerdctl must execute;
- BuildKit must be active and return workers.

The desktop first-run path also finishes with both `hyperv health` and `hyperv ps`.

## Desktop timeout/output handling — REMEDIATED substantially

Tauri command capture is now bounded to 256 KiB per stream and timed-out child processes receive a finite five-second termination/reap grace period.

The former indefinite timeout loop and unrestricted `read_to_end` buffering are gone.

## Image integrity — REMEDIATED

Downloaded VM images require SHA-256/SHA-512 verification.

For the default Debian image, the Windows init path resolves the official SHA-512 manifest and verifies the image before promotion.

Custom downloaded image URLs require an explicit trusted digest.

## SMB password-at-rest storage — REMEDIATED

`HyperVConfig.smb_password` is skipped during JSON serialization.

Windows stores the separate secret as a CurrentUser DPAPI-protected blob (`smb-password.dpapi`) and migrates legacy plaintext JSON credentials.

---

# Remaining release blockers and high-severity gaps

## RB-01 — The exact ZIP is missing release files that the validation record says are delivered

**Severity: Blocker / release-integrity**

The exact uploaded `(8)` ZIP contains no:

- `.github/workflows/release-windows.yml`
- `.gitignore`

Yet:

- `VALIDATION.md` says the workflow exists and passed YAML parsing.
- `VALIDATION.md` says the patch contains that workflow.
- `DEPLOYMENT_PHASES.md` lists both files as delivered.
- `README.md` directs the release process to that workflow.

Comparison against the previous remediation payload confirms that those two files were part of the patch but were dropped from the current ZIP.

This is not merely cosmetic. The workflow was the intended protected build/sign/provenance/attestation contract. Even when signing is excluded from today's decision, the artifact still does not match its own validated source set.

**Required remediation:** package dotfiles/directories, assert their presence in the ZIP-generation gate, and hash the final source/handover archive against a release manifest.

---

## RB-02 — NSIS installs current-user by default, but StackDeck's installed-client scripts assume Program Files

**Severity: Blocker for NSIS client readiness**

`desktop/stackdeck/src-tauri/tauri.conf.json` configures NSIS but does not set `installMode`.

Tauri's default NSIS mode is `currentUser`, which installs under `%LOCALAPPDATA%`, not `C:\Program Files`.

StackDeck meanwhile hardcodes:

- `scripts/windows/08-Initialize-Installed-Runtime.ps1`
  - `$env:ProgramFiles\StackDeck\stackdeck-hive.exe`
- `scripts/windows/11-Verify-Installed-Client.ps1`
  - `$env:ProgramFiles\StackDeck`
- README
  - states MSI/NSIS payloads install under Program Files.

Therefore the NSIS package and the post-install recovery/qualification contract disagree.

**Impact:** a correctly produced NSIS client can install successfully but the supplied installed-runtime initialization and qualification scripts can fail because they search the wrong location.

**Required remediation:** either set:

`bundle.windows.nsis.installMode = "perMachine"`

or make all installed-client tooling resolve the real installed executable location rather than assuming Program Files.

---

## RB-03 — Clean-machine installed-client and live Hyper-V qualification remain explicitly unexecuted

**Severity: Blocker**

`VALIDATION.md` explicitly states that these remain external gates before client GO:

- clean-machine installed-client qualification;
- live Administrator-backed Hyper-V runtime qualification;
- VM/SSH/BuildKit/Compose/API/restart/DHCP/cleanup/registry/rollback scenarios.

It also records that live VHDX/Hyper-V initialization remains pending.

This is independent of Authenticode signing. It is direct functional evidence for the claim that a client installation works.

**Required remediation:** execute and archive the qualification matrix on clean supported client machines.

---

## RB-04 — The exact ZIP is source-only, not installable client media

**Severity: Blocker for the exact artifact**

The ZIP contains no:

- MSI;
- NSIS setup EXE;
- final Tauri `stackdeck.exe`;
- staged release sidecar.

It is a clean source-oriented archive.

Therefore the exact artifact cannot itself be described as "ready to be installed." It can only be described as a source release candidate from which installers may be built.

---

## H-01 — Installed `stackdeck-hive.exe` still exposes the legacy HTTP GUI on arbitrary bind addresses

**Severity: High security/wiring gap**

Both `stackdeck-cli` and `stackdeck-hive` are built from the same `crates/stackdeck-cli/src/main.rs`.

That command set still includes:

`gui --host <string> --port <port>`

The default is loopback, but there is no loopback-only enforcement.

`stackdeck-gui::run_server()` directly binds the supplied host and prints the management token. The browser GUI also places the token in query parameters/forms.

This conflicts with `SECURITY_REVIEW.md`, which says remote access to the StackDeck GUI is not supported by design.

The Docker API shim already has a much stronger remote-bind gate (`--allow-remote` plus mandatory token configuration); the GUI does not.

**Required remediation:** remove the legacy GUI command from the production hive binary, or enforce loopback-only binding and stop placing bearer material in URLs.

---

## H-02 — Installed-runtime recovery defaults assume a D: drive

**Severity: High reliability gap**

`scripts/windows/08-Initialize-Installed-Runtime.ps1` defaults to:

`D:\StackDeck\VMs\stackdeck-linux`

Other deployment paths default to `%SystemDrive%\StackDeck\...`, and the Tauri GUI falls back to the user's default StackDeck VM root if `D:` is absent.

A normal Windows workstation with only `C:` can therefore fail the installed-runtime helper unless the operator knows to override `-VmRoot`.

**Required remediation:** use `%SystemDrive%` or resolve the configured/recommended root consistently across desktop, deployment, recovery, and qualification scripts.

---

## H-03 — Standard-user UAC with separate admin credentials can split StackDeck state across profiles

**Severity: High client support gap**

The Tauri desktop runs as the interactive user, but first-run provisioning launches elevated PowerShell with `Start-Process -Verb RunAs`.

StackDeck's registry/config/SSH/DPAPI state is rooted in `USERPROFILE\.stackdeck_runner`.

If the interactive account is a standard user and UAC elevation is approved with a *different administrator account*:

- the desktop reads/writes the standard user's profile;
- the elevated `stackdeck-hive` reads/writes the administrator's profile;
- CurrentUser DPAPI secrets also belong to the elevated administrator profile.

The desktop can therefore return from a successful elevated provisioning attempt and still see a different/uninitialized configuration.

The same identity issue applies to user-scoped scheduled-task behavior.

**Required remediation:** either explicitly require the interactive StackDeck user to be a local administrator who elevates the same identity, or move machine/runtime state to a well-defined machine scope and design credential storage accordingly.

---

## H-04 — Runtime base image and bootstrap are integrity-checked but not immutable

**Severity: High release-reproducibility gap**

The desktop and installed-runtime script use:

`https://cloud.debian.org/images/cloud/bookworm/latest/...`

and fetch the matching live SHA-512 manifest at installation time.

The guest bootstrap also executes live package repository operations and fetches nerdctl artifacts from upstream.

This protects integrity against a corrupted download, but it does not make the runtime immutable: a client installing the same StackDeck 0.6.0 package later can receive a different Debian image/package set than the one originally qualified.

**Required remediation:** pin a tested Debian image version and digest in the release, pin/bootstrap repository expectations, or ship a release-controlled VHDX/image manifest.

---

## H-05 — Installer payload verification checks valid signatures, not exact payload identity

**Severity: High release-chain evidence gap**

`scripts/windows/10-Verify-Installer-Payloads.ps1`:

- extracts MSI and NSIS;
- finds embedded `stackdeck.exe` and hive executable(s);
- verifies their Authenticode signatures.

It does **not** compare the extracted files' SHA-256 hashes against the canonical signed desktop/staged sidecar files created immediately before bundling.

A stale but validly signed executable could therefore pass this verification.

**Required remediation:** before bundling record canonical signed payload hashes; after extracting each installer require exact hash equality for every expected executable and reject unexpected executable payloads.

---

## M-01 — SMB SecureString still becomes plaintext in the child CLI command line

**Severity: Medium**

`02-Configure-HyperV.ps1` accepts a `SecureString`, but converts it to plaintext and passes:

`--smb-password <plaintext>`

to the CLI process.

The CLI then protects it with DPAPI for storage, so the at-rest defect is fixed. However, the password is transiently visible in the process command line.

The security document acknowledges the raw CLI exposure, but the SecureString deployment helper still eventually uses the same argv path.

**Recommended remediation:** pass the secret over stdin, named pipe, inherited handle, or Windows Credential Manager reference rather than argv.

---

## M-02 — Legacy desktop/browser GUI implementations remain workspace members

The release client is Tauri, but the workspace still includes:

- `stackdeck-desktop` (legacy eframe desktop)
- `stackdeck-gui` (HTTP browser GUI)

They are not packaged by the Tauri installer, but they remain buildable source surfaces and the hive depends on `stackdeck-gui`.

This increases release ambiguity and keeps a deprecated management interface in the production sidecar command set.

---

## M-03 — Functional qualification is coupled to signature verification

`11-Verify-Installed-Client.ps1` always checks Authenticode as part of its client qualification.

That is correct for the final release, but when signing is intentionally excluded from a pre-signing GO decision there is no separate official "functional qualification without signature" mode.

A clean pre-signing install/runtime test therefore needs an explicit documented procedure or a `-SkipSignature` qualification mode that is forbidden in the final production gate.

---

# Static and structural validation performed on `(8)`

| Check | Result |
|---|---|
| ZIP CRC/integrity | PASS |
| ZIP path traversal | PASS |
| ZIP symlink scan | PASS |
| JSON parse | PASS |
| TOML parse | PASS |
| Workspace/package/Tauri version alignment | PASS — 0.6.0 |
| Rust source files present | 11 |
| PowerShell scripts present | 15 |
| Rust test annotations found statically | 192 |
| Prior patched PID/lock/SQLite source | PRESENT |
| Prior patched first-run readiness source | PRESENT |
| Prior patched strict runtime health | PRESENT |
| Prior patched bounded desktop process capture | PRESENT |
| Prior patched DPAPI SMB persistence | PRESENT |
| Prior patched build/sign order scripts | PRESENT |
| `.github/workflows/release-windows.yml` | **MISSING** |
| `.gitignore` | **MISSING** |
| MSI/NSIS installer in exact ZIP | **MISSING** |
| final `stackdeck.exe` in exact ZIP | **MISSING** |
| Rust compile/test in this audit environment | UNPROVEN — Rust toolchain unavailable |
| PowerShell parser execution in this audit environment | UNPROVEN — PowerShell unavailable |
| clean npm dependency install | UNPROVEN — audit package mirror returned 404 for `vite@8.1.5` |
| live Hyper-V VM qualification | **PENDING per project validation record** |
| clean-machine install qualification | **PENDING per project validation record** |

The npm failure is an audit-environment package-mirror limitation, not evidence that the lockfile itself is invalid.

---

# Windows support conclusion

Do **not** market this as generically "Windows 10/11 ready."

The code itself now correctly treats Windows 11 as the default support baseline and makes Windows 10 opt-in through `-AllowWindows10`.

Recommended release support statement after the remaining defects and qualification are complete:

**Windows 11 Pro / Enterprise / Education, x86-64, on hardware satisfying Hyper-V requirements, with Hyper-V enabled and the required StackDeck prerequisites installed.**

Windows 10 should be supported only for a specifically qualified client estate (for example an approved ESU/LTSC configuration), not as an unrestricted generic target.

---

# GO criteria remaining, excluding Authenticode signing itself

Before issuing a client GO even if signing is intentionally postponed:

1. Restore `.github/workflows/release-windows.yml` and `.gitignore` to the exact handover ZIP and make archive verification fail if either is absent.
2. Fix the NSIS install-mode/Program Files mismatch.
3. Normalize installed-runtime VM root discovery instead of assuming D:.
4. Decide and enforce the Windows identity model for UAC provisioning.
5. Remove or loopback-lock the legacy web GUI in the production hive.
6. Pin the runtime image/runtime bootstrap to a qualified immutable release input.
7. Add exact embedded-payload hash matching after MSI/NSIS extraction.
8. Build unsigned functional MSI/NSIS artifacts from the exact commit and run clean-machine installation/repair/upgrade/uninstall tests.
9. Execute first-run VM creation, image verification, SSH, containerd, nerdctl, BuildKit, Compose smoke, API task, DHCP/restart recovery, portproxy/firewall cleanup, and rollback/replacement scenarios on supported Windows 11 hardware.
10. If Windows 10 is required, repeat the complete matrix on the exact supported Windows 10 ESU/LTSC population.

After those non-signing gates pass, the source can reasonably be classified as **functional client GO pending only final Authenticode signing/provenance publication**.

At present it is not yet at that point.
