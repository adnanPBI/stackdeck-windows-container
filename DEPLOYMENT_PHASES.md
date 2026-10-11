# Phase-by-phase local deployment package

This repository now includes a Windows 11 Pro/Enterprise/Education x86-64 local deployment package for the Hyper-V production MVP. Windows 10 is outside the default release baseline unless a specific ESU/LTSC client estate is separately qualified with `-AllowWindows10`.

## What was added

```text
scripts/windows/00-Ensure-Prereqs.ps1
scripts/windows/01-Build-Release.ps1
scripts/windows/02-Configure-HyperV.ps1
scripts/windows/03-Init-HyperV-Runtime.ps1
scripts/windows/04-Smoke-Test.ps1
scripts/windows/05-Install-Api-Task.ps1
scripts/windows/06-Diagnostics.ps1
scripts/windows/07-Sign-Release.ps1
scripts/windows/09-Build-Signed-Release.ps1
scripts/windows/10-Verify-Installer-Payloads.ps1
scripts/windows/11-Verify-Installed-Client.ps1
scripts/windows/Invoke-StackDeckLocalDeploy.ps1
docs/LOCAL_DEPLOYMENT_WINDOWS.md
docs/PRODUCTION_MVP_ARCHITECTURE.md
examples/smoke/docker-compose.yml
.github/workflows/release-windows.yml
.gitignore
```

## Source-level fixes included in this packaging pass

```text
Removed duplicate Hyper-V CLI windows_host field.
Removed duplicated Ok(()) expression in Hyper-V network helper.
Removed committed Tauri sidecar .exe files from source.
Added .gitignore for generated binaries, logs, and build output.
Added Windows deployment scripts and smoke-test compose file.
```

## Honest status

This is a release-candidate source package. It is not a client release until the protected signed-release workflow produces MSI/NSIS artifacts and clean-machine Windows/Hyper-V qualification evidence is archived.

## 2026-08-10 v8 client-GO remediation addendum

Additional release-safety files:

- `.github/workflows/release-windows.yml` — protected Windows release workflow.
- `.gitignore` — prevents stale generated payload/state artifacts from handover source.
- `runtime/windows-runtime-lock.json` — fail-closed immutable runtime input contract.
- `scripts/windows/12-Lock-Runtime-Image.ps1` — writes qualified Debian/snapshot/nerdctl release inputs.
- `scripts/windows/13-Verify-Source-Release.ps1` — enforces release source, NSIS, runtime-lock, legacy-GUI, and hygiene invariants.
- `scripts/windows/14-Package-Source-Handover.ps1` — packages source with dotfiles preserved and verifies required hidden paths after ZIP creation.

The release sequence is now: lock runtime inputs → verify source → build/test → sign canonical payloads → record canonical payload hashes → bundle MSI/NSIS → sign installers → extract and hash/signature-match embedded payloads → write checksums/provenance/attestation.

Clean-machine/live-Hyper-V qualification remains an evidence gate and is not represented as completed by source changes alone.
