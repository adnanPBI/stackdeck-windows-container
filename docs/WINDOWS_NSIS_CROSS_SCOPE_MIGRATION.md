# Windows v0.5.0 NSIS → v0.6.0 migration contract

**Release status:** A successful MSI major-upgrade or v0.6.0 clean NSIS install does **not** prove historical NSIS migration. Qualified Windows test evidence is mandatory.

## Why a same-place upgrade is unsafe

Authentic v0.5.0 used Tauri's historical NSIS default `currentUser`: executable and uninstaller in the signed-in user's `%LOCALAPPDATA%\StackDeck` and a registration in HKCU. StackDeck v0.6.0 intentionally uses `perMachine`: binaries under `%ProgramFiles%\StackDeck`, registry under HKLM. Tauri does not automatically map a per-user installation to a machine-wide installation, and the UAC administrator may be **a different account** from the user who originally installed v0.5.0.

The v0.6.0 NSIS preinstall hook blocks the new package when it detects the current account's historical uninstall registration or `%LOCALAPPDATA%\StackDeck\uninstall.exe`. This is **intentional**: it prevents overwriting or misreporting mixed-scope installs. It does not delete the legacy installation, files, credentials, or VM.

## Migration procedure for a verified v0.5.0 user

1. Sign in as **the same Windows account** that installed v0.5.0. Exit StackDeck and its Hive process.
2. Run `powershell -NoProfile -File scripts/windows/20-Inspect-Legacy-NSIS-Upgrade.ps1` to identify the current user's installation scope without modifying anything. This does not enumerate other user profiles.
3. Back up the original user's StackDeck application data/configuration and any deployment definitions to an access-controlled, independently verified location. Never copy plaintext SSH keys, passwords or SMB secrets to an open shared folder. Retain protected VM disks, approved VM configuration, and Hyper-V exports separately.
4. Use **Windows Settings → Apps → Installed apps → StackDeck → Uninstall** while signed in as the original user. Confirm the historic HKCU uninstall entry and `uninstall.exe` are no longer present. If either remains, stop and investigate; do not manually delete arbitrary folders or registry keys.
5. Run the migration inspection again and require `NO_CURRENT_USER_LEGACY_CONFLICT`. If any other account installed StackDeck, that account needs its own migration review before the machine-wide installer is accepted.
6. Install the **signed** v0.6.0 MSI or machine-wide NSIS via the authorized administrator. If elevated with a *different* account, do not assume the old user's HKCU installation has been removed.
7. Verify a single v0.6.0 registered installation, the correct installed binary SHA-256, and functional launch. On a properly privileged account, verify Hyper-V/SSH/containerd/nerdctl/BuildKit, Compose, HTTP 200, cleanup and machine restart recovery.
8. Record old and new installer hashes, actual uninstall/upgrade exit codes, before/after config digests, preserved user files, VM GUID and full provenance. Sign off only on a separate authorized clean Windows 11 client.

## Release criteria and limitations

- Cross-scope migration is explicitly **uninstall previous per-user installation, then install machine-wide**. It is **not** an automatic in-place NSIS upgrade.
- A different user's HKCU hive is not visible to the elevated installer. Do not assume cross-account global detection. Administrators should inventory authorized accounts with the owner's participation rather than mounting/deleting unrelated user hives.
- Historical v0.5.0 NSIS was unsigned. Verify its authentic historical package checksum before using it in a lab; do not instruct users to disable production Windows malware protection or SmartScreen.
- The v0.6.0 NSIS preinstall hook must be tested in both attended and silent modes; the latter must terminate nonzero on a detected old installation.
- An incomplete NSIS run, duplicate installation, changed user configuration or VM identity is a release-blocking failure. Restore the qualified MSI from verified backups before attempting another test.
- No new VM, especially `pystack-linux`, is permitted on the RND qualification workstation.
- Do not merge, tag or publicly distribute until independently accepted, publicly trusted signed installers and a human-approved exact source/evidence digest exist.
