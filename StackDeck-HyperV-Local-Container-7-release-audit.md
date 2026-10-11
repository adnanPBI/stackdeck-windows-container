# StackDeck Hyper-V Local Container (7) — Deep Client Release Audit

**Audit date:** 2026-08-09
**Artifact audited:** `StackDeck-HyperV-Local-Container(7).zip`
**ZIP SHA-256:** `0f01410b48ba5ac4005ee2030d1398088682e191e5e530fe5ecb3df2f7f90307`
**ZIP size:** 5,082,988 bytes

> Important scope note: the uploaded artifact is the **StackDeck Hyper-V Windows project**, not the fintech payment-aggregator package. This report applies to the exact uploaded StackDeck ZIP.

## Final verdict

**NO-GO for client release.**

I cannot certify the attached ZIP as a client-ready Windows 10/11 release or state that it will function correctly immediately after installation.

The package contains meaningful fixes to earlier defects, but it still has multiple independent release blockers in:

- release packaging/signing;
- the Windows release build script;
- build/sign artifact path wiring;
- installer payload signing order;
- first-run runtime readiness detection;
- first-run health finality;
- timeout/output handling;
- client prerequisite handling;
- local deployment documentation;
- Windows support qualification;
- live Hyper-V and clean-machine installation evidence.

The exact attached ZIP is also a **source package**, not installable client media: it contains no MSI, NSIS setup executable, or Tauri `stackdeck.exe`.

---

# Executive findings

| Severity | Finding | Client consequence |
|---|---|---|
| Blocker | Exact ZIP contains no client installer | Nothing in this ZIP can be installed as the finished StackDeck desktop client |
| Blocker | Claimed signed release workflow is absent | Signing/provenance pipeline cannot be reproduced from the delivered tree |
| Blocker | `01-Build-Release.ps1` references undefined variables under StrictMode | Successful build can terminate as a PowerShell failure |
| Blocker | Build and signing scripts disagree on desktop/bundle output root | Signing script can fail to find the artifacts produced by the documented release build |
| Blocker | Manual release order bundles before payload signing | Signed MSI/NSIS can still contain unsigned embedded application/sidecar payloads |
| Blocker | Desktop “ready” status detects an image-cache VHDX, not a working VM | Failed first run can be falsely marked Ready and hide the provisioning action |
| High | Desktop first-run ends with `hyperv health`, which does not fail on unhealthy runtime state | UI can report setup completion without the stronger runtime usability gate |
| High | Desktop timeout runner can loop indefinitely after timeout and buffers unbounded output | Provisioning can hang or consume excessive memory |
| High | Clean client install depends on external qemu-img and Hyper-V prerequisites | Fresh supported Windows PC is not automatically ready after installing |
| High | README quickstart omits the required trusted image digest | Documented manual initialization fails deterministically |
| High | Windows 10/11 support statement is overly broad | Home editions are unsupported; standard Windows 10 is beyond normal support in 2026 |
| Blocker | Live Hyper-V / clean-machine / signed installer evidence remains explicitly pending | Correct behavior after client installation has not been demonstrated |
| Medium | SMB password is serialized in plaintext configuration | Host credential exposure if SMB bind mounts are enabled |
| Medium | Included sidecars are unsigned generated binaries | Release hygiene and trust evidence are inconsistent |
| Medium | Provenance generation requires Git/GITHUB_SHA but this ZIP has no `.git` | Manual source-ZIP provenance generation fails without injected commit metadata |

---

# Release blockers in detail

## RB-01 — The attached ZIP is not an installable client release

The archive contains source, scripts, frontend build output, and two prebuilt sidecars, but contains no:

- `.msi`
- NSIS setup executable
- final Tauri `stackdeck.exe`
- signed checksum manifest
- provenance document
- release attestation

The only Windows executables in the ZIP are:

- `desktop/stackdeck/src-tauri/bin/stackdeck-hive.exe`
- `desktop/stackdeck/src-tauri/bin/stackdeck-hive-x86_64-pc-windows-msvc.exe`

Both are the same x86-64 PE and are not Authenticode signed.

Therefore this exact ZIP cannot be described as client install media.

---

## RB-02 — `VALIDATION.md` claims a signed workflow that is not delivered

`VALIDATION.md:180-186` states that:

` .github/workflows/release-windows.yml `

is delivered and performs the intended signing/provenance sequence.

The uploaded ZIP contains **no `.github` directory at all**.

This is both a release-integrity and documentation discrepancy.

Additionally, `VALIDATION.md:190-198` explicitly states that release remains blocked pending:

1. execution of the protected signed Windows release workflow with the real certificate; and
2. administrator-backed live Hyper-V/runtime qualification.

The project's own validation record therefore does not certify release readiness.

---

## RB-03 — Fatal end-of-build variable defect

`scripts/windows/01-Build-Release.ps1` enables:

`Set-StrictMode -Version Latest`

It defines:

- `$StackDeckCliOutput`
- `$StackDeckHiveOutput`

But at lines 134-135 prints:

- `$StackDeckOutput`
- `$HiveOutput`

Those variables are never defined.

Under StrictMode, the script can reach the end after successfully producing artifacts and then fail while reporting the result. The one-command deployment wrapper treats that as a failed build.

This must be corrected before the build script can serve as a release gate.

---

## RB-04 — Build/sign output directories are inconsistent

`01-Build-Release.ps1` forces:

- `CARGO_TARGET_DIR = <repo>\target`
- `CARGO_BUILD_TARGET = x86_64-pc-windows-msvc`

and treats the desktop bundle root as:

`target\x86_64-pc-windows-msvc\release\bundle`

It copies only the CLI and hive compatibility executables into:

`target\release`

However, `07-Sign-Release.ps1` hardcodes:

- desktop: `target\release\stackdeck.exe`
- bundles: `target\release\bundle`

Therefore the documented build path and signing path do not share one canonical artifact location.

A release should have one immutable output root used by build, payload verification, signing, bundling, manifest generation, and publication.

---

## RB-05 — Manual signing order can leave unsigned payloads inside signed installers

`desktop/stackdeck/package.json` defines:

`package:windows = build hive -> stage hive -> build desktop -> verify desktop -> bundle MSI/NSIS`

The README then tells the releaser to:

1. run `npm run package:windows`;
2. sign the application and sidecar;
3. sign MSI and NSIS.

At step 1, the MSI/NSIS already captured the then-unsigned executable payloads. Signing the standalone executable files afterward does **not** rewrite those bytes inside already-built installers.

`VALIDATION.md` describes the correct intended sequence:

1. build;
2. sign desktop payload and sidecar;
3. bundle;
4. sign installers.

But the workflow that allegedly implements that sequence is missing from the ZIP.

This is a serious release-chain wiring issue.

---

## RB-06 — First-run readiness detects the cached base image, not the runtime VM

In `desktop/stackdeck/src-tauri/src/main.rs`:

`runtime_setup_status()` sets:

`ready: vhdx_path.is_some()`

where `find_runtime_vhdx()` searches only:

`<vm_root>\images`

for any `.vhdx`.

During normal initialization, the downloaded Debian QCOW2 is converted to a VHDX under that `images` directory **before** VM creation, IP discovery, SSH readiness, and container runtime bootstrap finish.

The actual VM disk is separately copied to:

`<vm_root>\<vm_name>.vhdx`

Therefore this sequence can occur:

1. download succeeds;
2. image conversion succeeds;
3. cached base `.vhdx` now exists in `images`;
4. VM creation/bootstrap fails;
5. application restarts;
6. `runtime_setup_status()` sees the cached image and returns `ready = true`;
7. the UI disables VM-root/disk inputs and hides the “Create and initialize runtime” button.

The UI can therefore become stuck in a false-ready state after partial provisioning.

**Required fix:** readiness must verify the intended VM exists, its attached VM disk exists, VM state is valid, SSH works, containerd is active, nerdctl works, and the runtime health contract passes.

---

## RB-07 — Desktop first-run does not use the strongest success gate

The elevated first-run script ends with:

`stackdeck-hive hyperv health`

But `runtime_health_check()` only returns a JSON-like health map. It can report:

- SSH = unreachable
- containerd inactive
- BuildKit unavailable

while still returning `Ok(...)`.

The CLI `hyperv health` consequently exits successfully after printing the bad status.

By contrast:

- `03-Init-HyperV-Runtime.ps1`
- `08-Initialize-Installed-Runtime.ps1`

both run:

`hyperv ps`

after `hyperv health`, with the explicit comment that `hyperv ps` is the final gate because it fails if SSH/containerd/nerdctl is not actually usable.

The Tauri first-run flow omits this final gate.

**Required fix:** make a strict health command that returns non-zero on failed runtime invariants and use it for desktop completion.

---

## RB-08 — Desktop command timeout is not a hard timeout and output is unbounded

`run_command_with_timeout()` in the Tauri backend:

- spawns stdout and stderr reader threads;
- calls `read_to_end()` into unrestricted `Vec<u8>` buffers;
- truncates only after command completion;
- when the timeout is reached, repeatedly invokes tree-kill/child-kill;
- does not break from the loop or impose a post-kill deadline.

If termination fails, the loop can continue indefinitely despite having reached its timeout.

A verbose or stuck child can also accumulate unbounded output in memory.

**Required fix:** use bounded/ring buffers while reading, kill once, wait for a bounded kill grace period, and return timeout even if reap/termination encounters an error.

---

## RB-09 — Fresh installed-client provisioning depends on external tooling not bundled by the installer

The desktop default first-run downloads:

`debian-12-genericcloud-amd64.qcow2`

The Hyper-V layer requires `qemu-img` to convert QCOW2/IMG to VHDX.

The desktop installer configuration bundles the application, hive sidecar, and offline WebView2 installer, but does not bundle QEMU/qemu-img.

The installed first-run flow also assumes:

- supported Hyper-V Windows edition;
- hardware virtualization/SLAT;
- Hyper-V feature enabled;
- Hyper-V PowerShell tools;
- Default Switch present;
- administrator/UAC approval;
- SSH/SCP;
- internet access to Debian image/manifest;
- adequate local disk/RAM.

Thus “install MSI/NSIS and it works” is not presently a valid support claim.

---

## RB-10 — The README quickstart cannot initialize the default downloaded image as written

README lines 68-75 tell the user to run:

`03-Init-HyperV-Runtime.ps1`

without supplying either:

- `-Sha256`
- `-Sha512`

But the script explicitly throws at lines 77-79:

`Downloaded images require at least one trusted digest: -Sha256 or -Sha512.`

Therefore the root README's documented quickstart fails by design.

The same README also presents the SMB password as a plain command-line string while `02-Configure-HyperV.ps1` declares a `SecureString` and specifically instructs use of:

`(Read-Host -AsSecureString)`

The detailed deployment guide is more accurate than the root quickstart, but client release documentation needs one correct path.

---

## RB-11 — Windows 10/11 compatibility is not correctly bounded

The release code is hardwired to:

`x86_64-pc-windows-msvc`

so ARM64 Windows is not part of the delivered target.

The prerequisite script labels any returned Windows caption as `OK`; it does not explicitly establish a support matrix by edition/build/security-support status.

A valid release statement should be narrower:

- Windows 11 Pro/Enterprise on supported x86-64 hardware with Hyper-V requirements satisfied.
- Windows Home is not supported by the product's Hyper-V architecture.
- Windows 10 should not be advertised generically as a normally supported baseline in 2026; any retained Windows 10 support should explicitly identify the tested ESU/LTSC population.

---

## RB-12 — Required real Windows evidence is still missing

`VALIDATION.md` explicitly says the following remain pending:

- administrator Hyper-V lifecycle;
- SSH bootstrap;
- DHCP recovery;
- BuildKit;
- SMB mounts;
- portproxy/firewall cleanup;
- authenticated registry pull;
- clean-machine WebView2 installation;
- MSI/NSIS launch;
- Authenticode signing;
- provenance attestation.

Later sections explicitly state that no current VHDX was available for live initialization qualification.

Without those tests, it is not technically responsible to assert that the client “will function correctly after installing” on Windows 10/11.

---

# Security and reliability findings

## H-01 — SMB host password is persisted in plaintext JSON

`HyperVConfig` contains `smb_password: String`.

`HyperVManager::save_config()` serializes the entire structure to pretty JSON and writes it to disk.

No DPAPI/Credential Manager protection is used.

The default installed flow leaves SMB disabled, which limits immediate exposure, but enabling host bind mounts can persist an actual Windows credential in recoverable plaintext.

**Recommended fix:** store credentials in Windows Credential Manager or protect them with DPAPI and persist only a credential reference.

---

## H-02 — Generated unsigned sidecars are included despite release-hygiene guidance

README states generated binaries are release artifacts and should not be committed.

Nevertheless the source ZIP contains two generated hive executables. Both:

- are x86-64 PE files;
- report version 0.6.0;
- have identical SHA-256:
  `15d35e77f44b12bef81d6f57f1575e4afdf1f384c61ee90fb61ec1834c368c4c`;
- contain no PE Authenticode Security Directory.

The archive also includes generated frontend `dist/` files and an example `.stackdeck/state.db`.

These are hygiene problems and can create stale-artifact ambiguity.

---

## H-03 — Manual provenance generation assumes Git metadata unavailable in this ZIP

`07-Sign-Release.ps1` resolves provenance commit as:

- `$env:GITHUB_SHA`, or
- `git -C <repo> rev-parse HEAD`.

This source ZIP has no `.git` directory and no delivered workflow to inject `GITHUB_SHA`.

Thus the local signing/provenance flow cannot produce provenance from this ZIP without additional external commit metadata.

---

# Earlier defects that appear correctly remediated

The current source does contain sound fixes for several previously reported issues.

## PID-reuse force-kill protection — fixed in source

`crates/stackdeck-process/src/lib.rs`:

- validates persisted PID identity before graceful shutdown;
- **revalidates identity immediately before `taskkill /F`** after the grace delay;
- preserves persisted state rather than terminating when identity changed.

This addresses the prior PID-reuse kill window.

## Cross-process stale-lock unlink race — fixed in source

`crates/stackdeck-types/src/lib.rs` now:

- uses a stable `.lock` file;
- obtains an OS-backed exclusive file lock using `fs2`;
- never deletes the lock path in `Drop`;
- simply unlocks the file handle.

This removes the prior stale-lock-remover race.

## SQLite read-then-write upsert race — fixed in source

`crates/stackdeck-state/src/lib.rs` now uses a single SQLite:

`INSERT ... ON CONFLICT ... DO UPDATE`

rather than a separate read/merge/write sequence, and includes concurrent update tests.

These are meaningful improvements and should be preserved.

---

# Static validation performed on the uploaded artifact

| Check | Result |
|---|---|
| ZIP path traversal | PASS |
| ZIP symlink inspection | PASS — none present |
| Cargo TOML parsing | PASS |
| Tauri/package/capability JSON parsing | PASS |
| Version alignment in workspace/Tauri/package | 0.6.0 |
| Included sidecar architecture | x86-64 |
| Included sidecar version strings | 0.6.0 |
| Included sidecar Authenticode certificate table | ABSENT |
| Root `.github` release workflow | **MISSING** |
| MSI/NSIS installer in ZIP | **MISSING** |
| Final Tauri `stackdeck.exe` in ZIP | **MISSING** |
| Rust compilation/tests in this audit environment | UNPROVEN — Rust toolchain unavailable |
| PowerShell parser execution | UNPROVEN — PowerShell unavailable |
| Frontend clean `npm ci` | UNPROVEN — audit package mirror returned 404 for Vite |
| Live Windows Hyper-V lifecycle | UNPROVEN — no Windows/Hyper-V host available |
| Clean-machine MSI/NSIS install | UNPROVEN — installers not supplied |
| Signed installer verification | UNPROVEN — signed installers not supplied |

The inability to execute Windows/Hyper-V gates in this audit environment is not itself a defect in the project. It means those gates must remain **unproven**, not silently promoted to PASS.

---

# Supported Windows release recommendation

Do not publish the current package as “Windows 10/11 compatible” without qualification.

Recommended initial supported client baseline:

**Windows 11 Pro/Enterprise x86-64**, with:

- compatible CPU virtualization features and SLAT;
- Hyper-V enabled;
- required Hyper-V management PowerShell capability;
- sufficient RAM/storage;
- qemu-img supplied by the product or installed through a controlled prerequisite;
- clean-machine MSI/NSIS qualification.

If Windows 10 support is commercially necessary, test and document a precise still-supported Windows 10 ESU/LTSC matrix rather than claiming all Windows 10 systems.

---

# Minimum release-approval checklist

Before changing the verdict to GO:

1. Fix `01-Build-Release.ps1` undefined variables.
2. Normalize release output directories across build, verify, sign, bundle, and manifest scripts.
3. Restore the signed Windows CI workflow and enforce the correct order: build → sign payloads → bundle → sign installers → extract/verify embedded payload signatures → manifest/provenance/attestation.
4. Generate the actual MSI and NSIS installer artifacts.
5. Correct first-run readiness to inspect the real VM/runtime, not cached image VHDXs.
6. Use a strict `hyperv ps`/strict-health final gate in desktop provisioning.
7. Make desktop child-process output bounded and timeout termination finite.
8. Bundle/manage qemu-img or create an explicit prerequisite installer/check before first-run.
9. Correct README quickstart digest and SecureString instructions.
10. Add explicit supported Windows edition/build/architecture checks.
11. Move SMB credentials to DPAPI/Credential Manager.
12. Run pinned Rust/Node builds from a clean clone.
13. Run all Rust tests and PowerShell parser checks.
14. Build installers on protected CI.
15. Authenticode-sign desktop EXE, sidecar, MSI, and NSIS with timestamping.
16. Extract both installers and verify their embedded executable payload signatures and hashes.
17. Execute clean Windows 11 install/upgrade/repair/uninstall tests.
18. Execute live Hyper-V create/bootstrap/compose/API/restart/DHCP/cleanup/recovery tests.
19. If Windows 10 remains supported, run the same matrix on the explicitly supported Windows 10 ESU/LTSC editions.
20. Publish hashes, provenance, and attestation for the exact installer binaries.

# Conclusion

**The corrected source is improved, but it is not client-release-ready yet.**

I would not give this artifact to a client as a finished local Windows installer, and I would not state that it is guaranteed to function correctly after installation.

The safest characterization today is:

> **Source-level release candidate with meaningful remediation completed, but blocked on several additional code/wiring fixes plus signed-installer and live-Windows qualification.**
