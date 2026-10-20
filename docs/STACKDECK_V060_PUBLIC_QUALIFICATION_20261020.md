# StackDeck v0.6.0 — verified evaluation preview (20 October 2026)

**Distribution status: UNSIGNED EXPERIMENTAL PREVIEW, not a stable production release.**

- [Official evaluation prerelease](https://github.com/adnanPBI/stackdeck-windows-container/releases/tag/preview-v0.6.0-c9607be36cc9)
- [Independent Windows GitHub Actions run — success](https://github.com/adnanPBI/stackdeck-windows-container/actions/runs/39409784504)
- [Immutable preview source](https://github.com/adnanPBI/stackdeck-windows-container/commit/c9607be36cc9acb14bd13020625b02a7a9838e9e)
- [Stable production acceptance tracking](https://github.com/adnanPBI/stackdeck-windows-container/issues/8)

## What StackDeck does

StackDeck provides a **native Windows desktop (Tauri/React) and Rust CLI** for running OCI containers through a controlled Debian guest on **Hyper-V**, with `containerd`, `nerdctl`, and `BuildKit`. It does not depend on Docker Desktop, WSL2, or a host Docker Engine. It is a **Docker Desktop alternative for the tested workflows**, not a drop-in promise of all Docker Engine API or Docker Compose behavior.

Tested local features include image pull/inspection/removal; named volume and bridge network lifecycle; container creation/start/stop/removal; a Dockerfile/BuildKit build; Compose startup, health and teardown; and a Windows HTTP 200 test. A **limited, bearer-token-protected Docker Engine API-compatible subset** is available on loopback. Published host port forwarding is TCP-only. The pinned nerdctl guest does not offer full Docker `unless-stopped` semantics; StackDeck has bounded, project-scoped teardown recovery rather than global prune.

## Actual qualification results

| Test | Result | Boundaries |
| --- | --- | --- |
| Locked Rust workspace | **PASS — 124 passed, 0 failed** | RND; also locked Rust/source checks passed on independent GitHub runner |
| Build: release Rust Hive, Tauri desktop, frontend | **PASS** | Final local runtime binary below; clean-runner rebuild from immutable preview tag |
| MSI and NSIS packaging | **PASS** | Independently built and embedded payload hashes checked on clean Windows runner |
| Historical NSIS installation safety guard | **PASS** | Simulated current-user legacy collision rejects; clean path allows; not a real cross-user upgrade |
| Synthetic support-log redaction | **PASS** | CI and RND; review any support bundle before sharing |
| Nginx/Compose lifecycle and Windows HTTP | **PASS** | Candidate passed 3 cycles; corrected *installed* Hive passed the exact smoke test twice, removing its container and owned network |
| Installed unattended API task | **PASS under current boot** | S4U service start/stop, HTTP 200 with bearer and 401 without token; no full reboot claim |
| Hyper-V approved guest preservation | **PASS locally** | Only the existing approved `stackdeck-linux` VM was exercised |
| External encrypted backup ciphertext verification | **PASS — 13/13** | On existing off-device share; key independently escrowed: **NO** |
| Authenticated restore-readiness probe | **PASS — 5/5 VM export components** | AES-GCM decrypt stream + original plaintext SHA-256 in memory; **not** a completed full isolated restore |
| Latest S4U post-boot logic dry run | **PASS** | Correctly reports `actual_host_restart_observed=false` |
| Actual Windows host reboot + unattended workload/portproxy recovery | **NOT QUALIFIED** | Active Excel and browser work; remote agent automatic reconnect not proven |
| Fresh independent Windows 11 install/repair/upgrade/uninstall | **NOT QUALIFIED** | MSI/NSIS build is not installation acceptance |
| Publicly trusted, timestamped DigiCert publisher signing | **NOT AVAILABLE** | Installers/executables are **NotSigned** |
| Stable `v0.6.0` release and human GO approval | **NO-GO** | Protected `main` and stable tag are untouched |

## Traceability: RND runtime versus independent preview binaries

The fully exercised installed RND Hive has SHA-256
`4625a697498362fc6cf51e137988cea722304dbe2cf5f79daccef9bad051fc3c`,
built from runtime-fix commit `14aa4a65a184c83c0d422407ac34bc5f3e853626`.
The public preview uses **new, independently compiled binaries** from the exact tag
`preview-v0.6.0-c9607be36cc9`, which includes subsequent NSIS QA harness corrections.
Reproducible *source and tests* are qualified; byte identity of cross-machine compilers
is **not** claimed.

The published unsigned MSI SHA-256 is
`6c3b711a561de05d9bba1ac97f641c40934fb019105350b49eae9c714381fca6`.
The published unsigned NSIS SHA-256 is
`19c8e6bbb10b78c48a04dd1ab8f7051d7f45bfc429c4f1d123c9f57b0812a369`.
Use `SHA256SUMS.txt` on the [GitHub release](https://github.com/adnanPBI/stackdeck-windows-container/releases/tag/preview-v0.6.0-c9607be36cc9); these checksums detect corruption, **not** publisher identity.

## Hash references for privately retained RND evidence

The following hashes bind private machine-specific reports. Do **not** commit their raw contents, recovery key, host IPs, Windows user identity, credentials, or original backup/export files.

| RND report under `target/release-evidence/` | SHA-256 |
| --- | --- |
| `20261020-installed-smoke-regression-v2.json` | `01b1f3130c19b6c17a37b785dad2ccf601e70527282cd05307b47255449801ff` |
| `20261020-candidate-compose-cleanup-live-v7.json` | `87d3ae710d4b2ac4f5c1a0f1ca31da1cf8c22ebf09f286e9870de448694c3507` |
| `20261020-qualified-installed-hive-cutover-v2.json` | `ebac488559139edec124690706996c988f5b7f20162f32c88cb279b70de8a317` |
| `20261020-local-go-zip-independent-integrity.json` | `63ef93e5f1007d7b1a63abf47342f57f906c00d36947c29ab2094c60f904c6c5` |
| `20261020-offdevice-backup-current-hashes.json` | `1efdfdb3e5f8fa699343f5da7a230c517c627035b7483b49f97d3b8bdb5ede9c` |
| `20261020-full-approved-vm-export-restorability.json` | `a154f5cf97eb7952c3a5b4993780e2e4fd383e6be37882e74f156b00c72779da` |
| `20261020-postboot-dryrun-qualification.json` | `42e4b586326995328d5bc3e6d33fcae14db6ed7bd34ff1a67296ce0f8e880dab` |
| `20261020-real-host-restart-preflight-v2.json` | `d17fd3a5c8e4fce115ae16f697ae7ee9a56f6fb20a8dab3d4e89ce1f44777345` |

An earlier RND decision record (preliminary, before the last CI tag) was
`20261020-stackdeck-release-decision.json`, SHA-256
`c82bb40bacf8aff042d0f26cfff41f4bda2ba1edf8719820551dd08270ca137f`.
That record's `new_preview_ci_pass_pending` field reflects **its creation time**, not
the now-successful independent run linked above.

## Safe evaluation and error reporting

Supported evaluation baseline: Windows 11 Pro/Enterprise/Education x64, working Hyper-V,
administrator rights for first-time VM setup, sufficient capacity, and disposable or
verified backed-up workloads. Install **either MSI or NSIS**, not both. Do not disable
SmartScreen or organizational security to run unsigned software. Never repurpose a
host containing unknown, valuable or unbacked-up VMs.

Run these from **elevated PowerShell** after initializing the runtime on a dedicated
test host:

```powershell
& "C:\Program Files\StackDeck\stackdeck-hive.exe" hyperv health
& "C:\Program Files\StackDeck\stackdeck-hive.exe" hyperv ps
# From a checked-out source tree:
.\scripts\windows\04-Smoke-Test.ps1 -StackDeckExe "C:\Program Files\StackDeck\stackdeck-hive.exe"
```

For failures, [use the support collector](SUPPORT_AND_TROUBLESHOOTING.md):
`21-Collect-StackDeckSupport.ps1 -IncludeLogs -IncludeEvents`.
Extract and **review/redact** the generated ZIP before uploading it to an issue;
do not share tokens, private SSH keys, `hyperv.json`, VM disks, or recovery keys.

## What is needed for production GO

An independent fresh-profile Windows 11 MSI + NSIS install/upgrade/repair/uninstall
acceptance; a controlled host restart with reliable remote reconnect and proof of
SSH/containerd/BuildKit, workloads, volumes and Windows HTTP/portproxy recovery;
independently escrowed backup key and an isolated full restore; a verified publisher's
publicly trusted timestamped Authenticode signatures for installers and embedded
executables; then explicit approval of the **exact source commit and evidence digest**.
Only after those gates may the protected stable-release workflow tag and publish `v0.6.0`.
