# StackDeck release-blocker remediation patch

This is a patch-only overlay for the StackDeck repository at baseline commit
`d0ba6728f8107653e3a07d833a0e12e20efd958d`. Extract it at the repository root
and allow the listed paths to replace their existing versions.

The patch combines the earlier Windows deployment-script hardening with changes
to the Rust runtime, Compose normalization, Docker API shim, Tauri bundling, CI,
and regression tests. `REMEDIATION-MATRIX.md` records the precise disposition of
each reported issue, including explicit fail-closed behavior where the current
Hyper-V architecture cannot safely emulate a Docker feature.

## Apply

```powershell
Expand-Archive .\StackDeck-release-blocker-remediation-patch.zip -DestinationPath . -Force
```

Review the overlay before committing it. The ZIP intentionally contains only
changed or new files; it is not a full repository.

## Required validation

Use Windows 11/Server with Hyper-V enabled and Rust 1.88:

```powershell
cargo fmt --check
cargo test -p stackdeck-api -p stackdeck-hyperv -p stackdeck-compose -p stackdeck-types -p stackdeck-state -p stackdeck-process --locked
cargo build -p stackdeck-cli --bin stackdeck-cli --bin stackdeck-hive --release --locked

Push-Location .\desktop\stackdeck
npm ci
npm run build
Pop-Location

.\scripts\windows\Invoke-StackDeckLocalDeploy.ps1 -Phase All `
  -SmbPassword (Read-Host "SMB password" -AsSecureString) `
  -Sha256 "<publisher image digest>"
```

Run the signed-release workflow with the required certificate secrets before
shipping an installer. A release remains blocked until the Windows/Hyper-V
integration run, clean-machine installer launch, adversarial Docker API suite,
and Authenticode verification all pass.

## Security-sensitive behavior changes

- Downloaded VM images require SHA-256 or SHA-512 and are verified before the
  partial file is promoted for conversion or use.
- SMB passwords are accepted by the scripts only as `SecureString`; guest
  credential files are copied over SSH instead of embedding passwords in guest
  shell commands.
- The Docker API requires `Authorization: Bearer
  <STACKDECK_DOCKER_API_TOKEN>` on loopback by default. The explicit
  `STACKDECK_DOCKER_API_REQUIRE_TOKEN=0` compatibility override is unsafe;
  non-loopback binding remains fail-closed unless authentication is enabled.
- Ordinary service start is idempotent. Existing containers are inspected and
  started in place; they are not unconditionally removed.
- Unsupported semantics return a validation error instead of being silently
  downgraded.
