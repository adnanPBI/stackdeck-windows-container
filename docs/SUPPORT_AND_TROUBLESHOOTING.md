# StackDeck for Windows — support, diagnostics and privacy

## Report an installed-version problem

You can send the **redacted support ZIP** using the included
`scripts/windows/21-Collect-StackDeckSupport.ps1` or the separate
`21-Collect-StackDeckSupport.ps1` source file provided with the release assets.
It works without Rust, Node, npm, the source repository, a project config or
administrative rights. It neither restarts the guest nor modifies any VM.

1. Reproduce the problem, note the local date/time and the exact command or screen.
2. Save the support script in a dedicated folder, then open PowerShell there.
3. Run this command for the usual diagnostic bundle:

   ```powershell
   powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\21-Collect-StackDeckSupport.ps1 -IncludeLogs -IncludeEvents
   ```

4. The script prints `STACKDECK_SUPPORT_ZIP=...` and SHA-256. By default the
   ZIP and matching `.sha256.txt` are saved on your Desktop.
5. **Extract the ZIP locally, review ALL its text, and remove any details you
   cannot share.** Redaction is best-effort and cannot guarantee that arbitrary
   application or third-party logs are free of confidential material.
6. Upload only the reviewed ZIP to your private support conversation or a
   private GitHub issue. Share the separate SHA-256 value if desired. For a
   public GitHub issue, prefer the exact error message and reproduction steps
   without attaching the ZIP.

**Basic metadata only**, without application logs or Windows events:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\21-Collect-StackDeckSupport.ps1
```

The ZIP includes OS/version metadata, installed executable hashes/signature
state, status of the **approved** Hyper-V VM if readable, issue template,
privacy notice, and (when opted in) bounded, sanitized StackDeck log tails
and relevant Windows Application installer events.

The collection does **not** include `C:\ProgramData\StackDeck\config`,
SSH private keys, recovery key material, a complete event log, API tokens,
raw databases, guest VHDX files, repository files or project-specific configs.
You must still review its contents. Tests cover synthetic credential, email,
IP and home-directory redaction; they do not guarantee perfect sanitization.

## Recommended details when asking for help

Give the StackDeck version, Windows edition/build, whether MSI or NSIS was
used, the UTC/local issue time, what you expected, what happened, and whether
the error occurs at installation, first launch, VM bootstrap, Compose,
port-forwarding, or cleanup. Also describe any installed-version upgrade path
(e.g. v0.5.0 NSIS current-user to v0.6.0 machine-wide).

### Installation / upgrade failure

When working directly with Windows Installer, capture a verbose **MSI** log:

```powershell
msiexec.exe /i ".\StackDeck_0.6.0_x64_en-US.msi" /L*v "$env:TEMP\StackDeck-MSI-install.log"
```

Then run the support collector with `-IncludeLogs -IncludeEvents`. The
collector automatically looks for recent `StackDeck*.log` files in Temp.
**Be careful:** rerunning a setup is an installation change; do not rerun it
on a critical machine just to get a log. Existing logs and the support
collector are safer first steps.

NSIS failures can be described using their exit codes and Windows Application
events. The source-controlled NSIS migration inspector is read-only:

```powershell
powershell.exe -NoProfile -File .\20-Inspect-Legacy-NSIS-Upgrade.ps1
```

### Guest or networking failure

Only on an already approved runtime, and only when it is safe to query:

```powershell
& "C:\Program Files\StackDeck\stackdeck-hive.exe" hyperv health
& "C:\Program Files\StackDeck\stackdeck-hive.exe" hyperv ps
```

If Windows reports an access error for VM state, try an elevated PowerShell
session; don't assume that a non-admin error means the Linux guest is down.
When reporting, do not paste raw `hyperv.json`, SSH keys, tokens or SMB
passwords. Replace private IPs and internal paths in any screenshot.

## Safety

- Never upload private SSH keys (`id_ed25519`, `id_rsa`), `.env` files,
  GitHub/DigiCert credentials, `hyperv.json`, backup recovery keys, account
  passwords, or unreviewed full log folders.
- Never delete or recreate a VM in response to an error message. Preserve
  backup and VM GUID identities.
- An unsigned installer may generate Microsoft Defender SmartScreen warnings.
  Do not bypass organizational security controls or recommend disabling
  SmartScreen. Use a verified publisher-signed package for general deployment.
- If your issue involves sensitive work/customer data, use a private support
  channel; public issues should include only redacted snippets.
