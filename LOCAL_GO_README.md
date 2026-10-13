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

That archive contains the MSI, NSIS installer, CLI/hive binaries, immutable runtime lock, SHA-256 manifest, provenance, first-run and installed-client qualification scripts, and a runnable Compose smoke example.

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

## Qualify directly from the extracted LOCAL-GO ZIP

The archive is also a standalone Windows **pre-signing test bundle**, without Rust/Node source tooling.
Use a disposable, dedicated **Windows 11 Pro/Enterprise/Education x64 Hyper-V test workstation**, not a host containing valuable registered VMs. Open an elevated PowerShell window, extract the ZIP, and work inside the extracted package root.

~~~powershell
Set-Location 'C:\StackDeck-Qualification'
.\qualification\16-Ensure-Prebuilt-Runtime-Prereqs.ps1
~~~

The gate must pass before installation. It checks Hyper-V, the Default Switch, SSH, and qemu-img. Install **one** of the unsigned MSI or NSIS packages from the installers directory, recording installer logs. Never publish or promote unsigned local test media.

For the MSI route, a test administrator can use:

~~~powershell
msiexec.exe /i '.\installers\StackDeck_0.6.0_x64_en-US.msi' /L*v '.\stackdeck-msi-install.log'
~~~

First-run initialization through the app's **VM** screen is supported. The matching scripted route automatically resolves the pinned runtime lock from the extracted package:

~~~powershell
.\qualification\08-Initialize-Installed-Runtime.ps1
~~~

The script refuses VM creation when the Hyper-V inventory cannot be enumerated, the chosen name is already registered, or a virtual disk already exists. It never automatically deletes an existing VM.

After the runtime is initialized and healthy, run:

~~~powershell
.\qualification\11-Verify-Installed-Client.ps1 -InstallRoot 'C:\Program Files\StackDeck' -SkipSignature -Output '.\installed-client-evidence.json'
.\qualification\04-Smoke-Test.ps1 -StackDeckExe '.\cli\stackdeck-cli.exe' -ComposeFile '.\examples\smoke\docker-compose.yml'
~~~

The Compose smoke test starts a test Nginx container, checks runtime status and localhost HTTP 200, and stops it afterward. Keep signed-release trust and MSI/NSIS repair/upgrade/uninstall checks separate. Repeat final installed-client qualification **without -SkipSignature** only after obtaining a trusted organizational code-signing certificate.

If any prerequisite is blocked, retain the diagnostic evidence. Do not delete or recreate an unknown VM to make a gate pass.
## Production/public release

The local package is deliberately **unsigned** and is for local functional qualification. A publishable client release must use `scripts\windows\09-Build-Signed-Release.ps1` or the protected `.github\workflows\release-windows.yml`, followed by clean-machine qualification **without** `-SkipSignature`.
