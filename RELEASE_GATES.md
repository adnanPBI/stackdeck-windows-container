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
7. Verify the extracted MSI and NSIS payload hashes using `scripts/windows/10-Verify-Installer-Payloads.ps1 -ExpectedPayloadManifest <manifest> -SkipSignature`. The signature bypass is only for local testing.
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

Configure code-signing secrets: `WINDOWS_SIGNING_CERT_PFX_B64`, `WINDOWS_SIGNING_CERT_PASSWORD`, and `WINDOWS_SIGNING_CERT_THUMBPRINT`. Use a currently valid publicly trusted code-signing identity that authorizes the publisher; self-signed developer certificates cannot qualify a public release. Keep the PFX and password out of source and logs.

The signed workflow runs locked tests, signs CLI/hive and desktop payloads *before* MSI/NSIS bundling, signs installers, extracts and verifies exact embedded hashes and Authenticode signatures, creates checksums and provenance, requests GitHub artifact attestation, and publishes exactly seven verified release assets. A manual workflow dispatch creates CI artifacts but does not create a public GitHub release.

## NO-GO conditions

Do not create/push the final `v0.6.0` tag or publish a public release if the machine-side Hyper-V tests, clean-client installation matrix, trustworthy certificate, release evidence SHA-256, or protected GitHub settings have not been independently verified. A local source candidate tag is not a production release.
