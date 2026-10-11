# StackDeck v0.6.0 — Local GO package

This source is prepared for the supported Windows 11 Pro/Enterprise/Education x64 + Hyper-V baseline. It does not use WSL/WSL2 or Docker Desktop.

## What was hardened in this GO source

- Restored the required protected Windows release workflow at `.github/workflows/release-windows.yml`.
- Retained the newer machine-scoped configuration design: `C:\ProgramData\StackDeck\config`, LocalMachine DPAPI, and private ACL boundary.
- Retained the immutable Debian/nerdctl runtime lock and fail-closed release checks.
- Retained per-machine NSIS installation, signed-payload verification, provenance, and clean-machine qualification gates.
- Removed disconnected legacy `stackdeck-gui` and `stackdeck-desktop` implementations from the release source so only the Tauri desktop control plane is shipped.
- Removed generated `desktop/stackdeck/dist` from source; `npm ci` + Vite regenerate it from the lockfile.
- Added `15-Build-Local-Go-Package.ps1` for a single-command unsigned local MSI/NSIS build with embedded-payload hash verification and artifact manifests.

## Build the complete local package

Open **Administrator PowerShell** in the repository root and run:

```powershell
Set-ExecutionPolicy -Scope Process Bypass -Force
.\scripts\windows\00-Ensure-Prereqs.ps1
.\scripts\windows\15-Build-Local-Go-Package.ps1
```

The complete local package is written under:

```text
target\local-go\StackDeck-v0.6.0-Windows-x64-LOCAL-GO.zip
```

That archive contains the MSI, NSIS installer, CLI/hive binaries, immutable runtime lock, SHA-256 manifest, provenance, and installed-client qualification scripts.

## Install and initialize

Before first runtime initialization, an installed/prebuilt client can run the runtime-only prerequisite gate without requiring Rust, Cargo, Node, or npm:

```powershell
.\scripts\windows\16-Ensure-Prebuilt-Runtime-Prereqs.ps1
```

Install **one** of the generated packages from `installers\` (MSI or NSIS). Launch StackDeck as the interactive local administrator, open **VM**, and select **Create and initialize runtime**. The default runtime downloads the pinned Debian cloud image, verifies its SHA-512, converts it to dynamic VHDX, configures Hyper-V, boots the VM, and bootstraps containerd/nerdctl.

If the default QCOW2 image is used, `qemu-img` must be available as documented in `docs\LOCAL_DEPLOYMENT_WINDOWS.md`.

## Qualify the local unsigned build

After runtime initialization, run from Administrator PowerShell:

```powershell
.\scripts\windows\11-Verify-Installed-Client.ps1 `
  -SkipSignature `
  -Output .\stackdeck-client-functional-qualification.json
```

Then execute the smoke and diagnostics gates described in `docs\LOCAL_DEPLOYMENT_WINDOWS.md`.

## Production/public release

The local package is deliberately **unsigned** and is for local functional qualification. A publishable client release must use `scripts\windows\09-Build-Signed-Release.ps1` or the protected `.github\workflows\release-windows.yml`, followed by clean-machine qualification **without** `-SkipSignature`.
