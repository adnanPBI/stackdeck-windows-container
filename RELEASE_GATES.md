# StackDeck v0.6.0 release qualification and publication

## Supported release baseline

Windows 11 Pro, Enterprise or Education x86-64, with working Hyper-V, Default Switch, administrator rights for first-run provisioning, verified Windows virtualization/SLAT, and qemu-img for the locked Debian QCOW2 image (or a qualified pre-converted VHDX). No WSL or Docker Desktop is required.

## Source and build gate

1. Keep all changes in an exact Git commit; do not use a dirty worktree for local GO or signed releases.
2. Run `scripts/windows/13-Verify-Source-Release.ps1` and `cargo fmt --check`.
3. Run `cargo test -p stackdeck-cli -p stackdeck-api -p stackdeck-hyperv -p stackdeck-compose -p stackdeck-types -p stackdeck-state -p stackdeck-process --locked`.
4. Build the two binaries using `cargo build -p stackdeck-cli --bin stackdeck-cli --release --locked` and the equivalent command for `stackdeck-hive`.
5. From `desktop/stackdeck`, run `npm ci`, `npm audit --audit-level=moderate`, and `npm run build`.
6. Prepare unsigned local test installers with `scripts/windows/15-Build-Local-Go-Package.ps1`. Never publish them as signed production media.
7. Verify the extracted MSI and NSIS payload hashes using `scripts/windows/10-Verify-Installer-Payloads.ps1 -ExpectedPayloadManifest <manifest> -SkipSignature`. The signature bypass is permitted for local functional testing and the separately labeled, hash-verified **unsigned preview workflow** only; it never constitutes production signature acceptance.
8. Export a committed source handover with `scripts/windows/14-Package-Source-Handover.ps1 -OutputZip <path>` after all intended files are committed.

## Independent clean-client qualification (Administrator)

Use a clean supported Windows 11 test machine or VM, not a production workstation with application data. Capture hostname, Windows edition/build, installation timestamps, command exit codes and sanitized logs in an archived evidence report.

- MSI: clean install; GUI launch without Vite development server; intact WebView2 runtime; first-run UAC; VM creation/initialization; repair; same-version reinstall; newer-version upgrade; uninstall; preservation of user data and existing unrelated Hyper-V resources.
- NSIS: repeat the same lifecycle. Confirm per-machine installation and Program Files path.
- Run `scripts/windows/16-Ensure-Prebuilt-Runtime-Prereqs.ps1` elevated without `-EnableFeatures` until prerequisites are reviewed.
- For the CLI path, use `02-Configure-HyperV.ps1 -SkipSmb` and `03-Init-HyperV-Runtime.ps1` on a dedicated test VM with no existing data, then `04-Smoke-Test.ps1`.
- Verify real SSH, containerd, nerdctl, BuildKit, image pull/build, Compose health/dependency ordering, API bearer-token enforcement and negative auth cases, portproxy/firewall publish and cleanup, DHCP IP change/reboot recovery, registry mirror removal, replacement/rollback, persistent state recovery, and disk/CPU/memory capacity.
- Run `11-Verify-Installed-Client.ps1 -SkipSignature -Output <evidence.json>` only for *pre-signing* functional qualification. Do not remove the signature check in production qualification.
- Preserve the sanitized evidence file and compute its SHA-256 digest. Both MSI and NSIS, runtime/rollback, and upgrade/uninstall must be independently marked PASS.

## Protected GitHub release workflow

The `.github/workflows/release-windows.yml` workflow can publish only from a release tag exactly equal to `v` plus the synchronized Cargo, npm and Tauri version. The checked-out tree must be clean. The protected runtime settings must exactly match the committed runtime lock; the workflow never rewrites release inputs.

Configure protected GitHub Environment `windows-release` and require human approval. Provide the following protected variables: `WINDOWS_RUNTIME_IMAGE_URL`, `WINDOWS_RUNTIME_IMAGE_SHA512`, `WINDOWS_DEBIAN_SNAPSHOT_TIMESTAMP`, `WINDOWS_NERDCTL_VERSION`, `WINDOWS_NERDCTL_SHA256_AMD64`, `WINDOWS_QUALIFIED_SOURCE_COMMIT`, and `WINDOWS_QUALIFICATION_EVIDENCE_SHA256`. The qualified commit must equal the exact tagged source commit, and the evidence digest must match the separately archived clean-client qualification report.

Use the **DigiCert KeyLocker** workflow in `docs/DIGICERT_KEYLOCKER_SIGNING.md`. Required protected secrets are `SM_API_KEY`, `SM_CLIENT_CERT_FILE_B64`, `SM_CLIENT_CERT_PASSWORD` and `WINDOWS_SIGNING_CERT_THUMBPRINT`; protected variables are `SM_HOST` and `WINDOWS_SIGNING_KEYPAIR_ALIAS`, in addition to the runtime/qualification variables above. The client-auth P12 is **not** the exportable signing private key. The publisher must enroll a publicly trusted, HSM-backed signing certificate, independently prove trust and remote-key custody, and verify a real timestamped Authenticode signing challenge. Never substitute the local self-signed developer certificate or print credentials to CI logs.

The signed workflow runs locked tests, signs CLI/hive and desktop payloads *before* MSI/NSIS bundling, signs installers, extracts and verifies exact embedded hashes and Authenticode signatures, creates checksums and provenance, requests GitHub artifact attestation, and publishes exactly seven verified release assets. A manual workflow dispatch creates CI artifacts but does not create a public GitHub release.

## Distinct UNSIGNED evaluation preview (not production GO)

For research and technically experienced testers only, a Git tag named `preview-v0.6.0-` plus the first 12 characters of the exact commit can trigger `.github/workflows/preview-windows.yml`. The workflow rebuilds Windows MSI/NSIS binaries from the tag, runs the locked Rust and packaging/payload tests, creates checksums and provenance, and publishes a **GitHub pre-release** explicitly identifying its assets as unsigned and experimental. It never publishes from a stable `v*` tag and never substitutes for the trusted signing workflow. Review `docs/UNSIGNED_PREVIEW_RELEASE.md` before distributing or recommending it. Do not bypass SmartScreen, Smart App Control or organizational policy; use a dedicated backed-up Windows 11 test host without valuable VMs. Release notes must disclose the missing independent fresh-profile and host-restart gates and any known installation compatibility gaps.

The presence of a GitHub pre-release is not a completed functional or security qualification. A source commit, artifact hash and automated test report are evidence of build provenance/integrity, not publisher identity or third-party trust. Do not market an unsigned preview as stable or suitable for production environments.

## RND functional qualification update — 2026-10-19

On the exact approved existing `stackdeck-linux` VM, installed-image/named-volume/bridge-network lifecycle (including data persistence through container recreation), an actual BuildKit Dockerfile build and HTTP 200, and the supported Docker REST API subset passed. The protected S4U startup task plus job-object child supervision passed a controlled stop/start test with exact API port release, bearer HTTP 200 and unauthorized HTTP 401. A distinct postboot verifier was armed and its current-boot negative test correctly refused to claim a real reboot. See `docs/RND_CONTAINER_AND_HOST_RESTART_ACCEPTANCE_20261019.md`.

**Still NO-GO for production:** RND had open Excel/browser processes and an interactive console; an actual host restart was not performed, so the full reboot, DHCP portproxy and workload HTTP recovery gates are pending. RND-specific scripts are not a generic supported installer for an independent clean Windows machine. The signed stable release, clean-client matrix and release approvals are unaffected and remain blocked.

## NO-GO conditions

Do not create/push the final **stable** `v0.6.0` tag or publish a production release if the machine-side Hyper-V tests, clean-client installation matrix, trustworthy certificate, release evidence SHA-256, or protected GitHub settings have not been independently verified. A preview tag is not a production release.
