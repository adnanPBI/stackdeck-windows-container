# Phase-by-phase local deployment package

This repository now includes a Windows 10/11 local deployment package for the Hyper-V production MVP.

## What was added

```text
scripts/windows/00-Ensure-Prereqs.ps1
scripts/windows/01-Build-Release.ps1
scripts/windows/02-Configure-HyperV.ps1
scripts/windows/03-Init-HyperV-Runtime.ps1
scripts/windows/04-Smoke-Test.ps1
scripts/windows/05-Install-Api-Task.ps1
scripts/windows/06-Diagnostics.ps1
scripts/windows/Invoke-PyStackLocalDeploy.ps1
docs/LOCAL_DEPLOYMENT_WINDOWS.md
docs/PRODUCTION_MVP_ARCHITECTURE.md
examples/smoke/docker-compose.yml
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

This is a local deployment-ready source package. It still requires local Windows validation because this environment cannot execute Hyper-V or compile Rust.
