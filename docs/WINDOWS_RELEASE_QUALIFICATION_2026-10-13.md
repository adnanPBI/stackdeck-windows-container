# StackDeck 0.6.0 Windows release qualification — 2026-10-13

## Decision
**Not approved for public release.** Source builds and offline installer payload integrity passed. Host runtime, clean-machine lifecycle and public signing remain unqualified.

## Exact workstation and source
- Windows 11 Pro x64, Windows version 10.0.26200, host RND.
- Source initially checked out at `0f453d8321d819f0b751ef13c5388fd94a87fc25` (`v0.6.0-rc.3`), initially clean.
- Rust 1.88; 104 scoped core tests, formatting and both CLI/hive release builds passed in the previous execution.
- Tauri desktop release executable includes the current hashed frontend asset and version-matching hive sidecar.
- `17-Elevated-Release-Preflight.ps1` writes JSON evidence under `target/release-evidence`.

## Package evidence (unsigned, NOT distributable)
| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `target/x86_64-pc-windows-msvc/release/bundle/msi/StackDeck_0.6.0_x64_en-US.msi` | 219131904 | `96CDA0060834BC9DA710E6A07E7D5D9038F652F0D86CA8A69985B11467EF69E2` |
| `target/x86_64-pc-windows-msvc/release/bundle/nsis/StackDeck_0.6.0_x64-setup.exe` | 220052449 | `98C8D0E96ED22F80877161A252B960C946FE52F8A3DB0B68FE043FF9EA498B86` |

Both installers were unpacked using 7-Zip 26.04. MSI nested CAB and NSIS payloads hash-matched the current canonical desktop and hive binaries. Signature checks were intentionally bypassed *only* for pre-signing validation.

## Privileged host discovery
- An approved `RunAs` PowerShell session ran and reported `Administrator=True`.
- `Microsoft-Hyper-V-All` is enabled, `vmms`, `vmcompute` and `hns` services are running; `Default Switch` exists.
- Hyper-V CIM lists two existing registrations: `stackdeck-linux` (ID `4ABC9A73-4FC8-44A2-9FD8-6A050101BEF8`) and `pystack-linux` (ID `6D657382-205E-40C4-83EE-E62B9776D43C`). Both show CIM EnabledState 6 (off).
- **Blocking:** `Get-VM` fails at `VMComputerSystemBaseView.get_Setting()` with `ObjectNotFoundException`. The PowerShell enumeration failure MUST NOT be interpreted as meaning no VMs are registered.
- Do not remove these registrations, disks, checkpoints or virtual switches without backup/recovery evidence and explicit approval.
- Pinpoint evidence: `target/release-evidence/hyperv-diagnostics.txt`. Diagnose `Msvm_VirtualSystemSettingData` association and on-disk VM configuration before repairing registration.
- `qemu-img` is required for Debian QCOW2 conversion; a winget installation was requested under an elevated process and must be verified before proceeding.
- Current user has locally trusted `CN=PyStack Container Self-Signed` Code Signing certificate. This is a development identity, **not** a trusted public release certificate.

## Safe host qualification sequence
1. On the local Windows console, run Administrator PowerShell. Do not reboot automatically.
2. Execute `./scripts/windows/17-Elevated-Release-Preflight.ps1` to record machine state. Use `-EnableFeatures` only if Hyper-V Windows features are disabled; restart manually only when reported necessary.
3. Verify `C:\Program Files\qemu\qemu-img.exe --version` and that complete 7-Zip is available. Run `./scripts/windows/16-Ensure-Prebuilt-Runtime-Prereqs.ps1`.
4. Investigate and preserve the two current VM registrations and backing files. Repair only after backed-up and verified; run `Get-VM` again to prove healthy enumeration.
5. On a designated clean Windows qualification VM/host, install **unsigned test media only**, verify desktop first launch, runtime initialization, SSH, containerd/nerdctl/BuildKit, Compose smoke, auth API, networking, DHCP/restart recovery and cleanup. Run `./scripts/windows/11-Verify-Installed-Client.ps1 -SkipSignature` and archive output.
6. Separately test MSI and NSIS clean install, repair, upgrade from approved prior version and uninstall; collect logs/screenshots/checksums. Avoid modifying any user production runtime during these tests.
7. Import a real organization-owned code-signing certificate into the protected release environment. Reject self-signed development certificates. Execute the signed CI workflow on **the exact qualified commit**.
8. Publish **only** after verifying signed embedded payloads, trusted Authenticode chains on a clean Windows machine, timestamp, SHA-256 provenance and GitHub attestation. The source release tag must equal the package version.

## Evidence separation
- Source/unit/build tests: completed.
- Unsigned MSI/NSIS structural and payload hash verification: completed.
- Elevated Hyper-V VM lifecycle and clean installed-client qualification: NOT completed.
- Independent clean-machine signature verification: NOT completed.
- GitHub public publication: NOT completed; this local Git repository does not have a remote.

## Subsequent RC5 safety remediation and standalone package improvements
- A dangerous legacy behavior in scripts/windows/08-Initialize-Installed-Runtime.ps1 was identified: stale VM disk detection previously triggered automatic Stop-VM and Remove-VM.
- This behavior has been removed. Initialization now reads Get-VM with ErrorAction Stop, cross-checks CIM registration and existing disks, then refuses to alter any registered or data-bearing VM.
- Both Rust VM creation entry points now run the same independent strict CIM/PowerShell/VHDX preflight before mutation and repeat their guard inside the actual create command to mitigate time-of-check/time-of-use races.
- New Rust regression test for the shared guard passed (26 Hyper-V crate tests).
- An elevated read-only test against the host's damaged stackdeck-linux registration returned a controlled fail-closed error; no test VM directory or config environment changes were observed. Evidence: target/release-evidence/rc5-hyperv-fail-closed-test.json.
- Standalone LOCAL-GO package updated to include the Compose smoke test script and its dedicated example, and to resolve the immutable runtime lock inside the extracted ZIP.
- The RND host remains unqualified for running VMs until the two orphaned Hyper-V registrations are separately recovered with backup evidence and signoff. None was deleted or reset.
- No publicly trusted publisher code-signing certificate has been provided; no signed public release is authorized.