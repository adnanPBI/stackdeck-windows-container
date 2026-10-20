# StackDeck v0.6.0 — UNSIGNED EVALUATION PREVIEW

> **Experimental, unsigned, not a stable production release.**
> This package is offered for technically experienced Windows 11 Hyper-V
> evaluators, not for unrestricted business deployment. Publisher-trusted code
> signing and independent clean-client/reboot qualification remain incomplete.

StackDeck is a native Windows container management tool: a Tauri desktop and
Rust CLI manage one designated Debian Hyper-V guest, which runs
containerd, nerdctl and BuildKit. The project is designed to operate without
Docker Desktop or WSL/WSL2; it is **not** a claim of complete Docker API
compatibility.

## What this preview includes

- Windows MSI and NSIS packaging, built from the exact tagged source commit on
  a clean GitHub Windows runner
- Native desktop executable, CLI, Hive sidecar, immutable runtime lock
- OCI/containerd/nerdctl/BuildKit runtime integration
- Docker Compose-style operations, test Nginx HTTP exposure and teardown
- Guarded NSIS cross-scope migration behavior: a per-user v0.5.0 installation
  must be backed up and removed by its original user before the v0.6.0
  machine-wide installer is used
- SHA-256 checksums, exact-source provenance and embedded-payload verification
- Opt-in redacted Windows support-log ZIP generator

## Proven evaluation and remaining limitations

On the authorized RND Windows 11 host, the corrected source passed 124 locked
Rust workspace tests (0 failed), frontend/desktop build, MSI and NSIS
packaging, and independent embedded-payload hash verification. The rebuilt
Hive was installed locally with a verified rollback backup and passed two
complete executions of `04-Smoke-Test.ps1`: authenticated guest SSH,
containerd, BuildKit, healthy Nginx with Windows HTTP 200, then confirmed
removal of the project container and its owned network. The candidate had
also passed three live Compose cycles and preserved the approved VM, unrelated
Docker API network and original VM backups. These are RND-only acceptance
results, **not** independent fresh-PC acceptance.

The pinned nerdctl 2.3.5 guest does not fully implement Docker's
`unless-stopped` creation semantics. StackDeck's guarded Compose shutdown
prevents resurrection during explicit teardown, but full restart-policy parity
should not be inferred. An actual unattended **Windows host reboot** has not
been qualified, and the RND machine had open interactive user work; the host
was not restarted without a safe maintenance window.

The following have **NOT** been independently certified on a separate fresh
Windows 11 machine: historical per-user NSIS v0.5.0 migration,
first-use VM provisioning and host-restart recovery. Do not assume a recorded
installer build equals a successful installation on every Windows system.

## Who should try it

- Developers using **Windows 11 Pro, Enterprise or Education x86-64** with
  Hyper-V enabled and appropriate administrator rights
- Users with a disposable or fully backed-up test computer
- Evaluators who understand the limitations of unsigned code and Windows
  Defender SmartScreen or Smart App Control

**Do not install on a Windows computer holding important/unknown Hyper-V
guests, production workloads, unbacked-up disks or sensitive secrets.** Do not
disable system-wide antivirus or Smart App Control to install this preview.
Enterprise policies may prohibit running unsigned binaries.

## Install / recover

1. Review the tagged source, checksums and provenance. Download only from the
   repository's official GitHub Releases page.
2. Verify the downloaded file SHA-256 with `Get-FileHash`. Compare with the
   release's `SHA256SUMS.txt`; authenticity is **not** established by a
   checksum supplied with an unsigned artifact.
3. If Windows blocks execution under your security policy, **do not defeat
   the security policy**. Build/review locally instead, or wait for a signed
   release.
4. Install **either** MSI or NSIS, not both. Never migrate a v0.5.0
   per-user installation to per-machine v0.6.0 without the documented backup,
   original-user uninstall and identity/registration checks.
5. Initialize only on a dedicated backed-up Windows Hyper-V evaluation host.
   Verify the VM identity before any provisioning. Never replace an existing VM
   disk or create a VM with a conflicting name.
6. Stop evaluation and collect a support bundle if any acceptance check fails.

## Bug reports

The release includes `21-Collect-StackDeckSupport.ps1`. For a privacy-filtered
ZIP with recent StackDeck logs and Windows Application installer events:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\21-Collect-StackDeckSupport.ps1 -IncludeLogs -IncludeEvents
```

It writes a ZIP and SHA-256 text file to the Desktop. **Inspect each extracted
file before sending it.** Do not include `hyperv.json`, SSH private keys,
passwords, tokens, disk images, full unreviewed logs or backup recovery keys.
See `docs/SUPPORT_AND_TROUBLESHOOTING.md` in the repository.

## Next official release

A stable, public-trust release still requires independently qualified
Windows 11 deployment, host restart and recovery, safe cross-account migration,
publisher authorization, publicly trusted timestamped Authenticode signatures
for the installers **and embedded executables**, and final human approval of
the exact commit and evidence digest.

This **pre-release never sets the production GO gate** or changes the protected
`v*` signing workflow.
