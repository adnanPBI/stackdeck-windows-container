# StackDeck public release: dedicated Hyper-V host and Authenticode signing

**Release status: NO-GO until the gates in this document pass.**
The main branch was replaced with StackDeck RC5 source commit
fdd75967d9971e3add442055a5ccbd27ddae0902 on 2026-10-15.
The unsigned RC5 LOCAL-GO ZIP is NOT a publicly trusted release.

## 1. Authorized, isolated Windows qualification host

Connect a separate Windows 11 Pro/Enterprise/Education x64 workstation using an
authorized remote-control connection. An elevated local administrator account
must be available, Hyper-V and its Default Switch must work, and qemu-img,
OpenSSH and sufficient disk space must be installed.

- Do not use the RND machine in its currently broken state. It has two
  pre-existing VM registrations, stackdeck-linux and pystack-linux.
  Get-VM fails even with an elevated token. Never reset, deregister, delete
  or repair those VMs without an independent verified backup and explicit
  recovery authorization.
- A public GitHub-hosted runner is not a substitute for stable Hyper-V
  qualification; nested virtualization is not officially supported there.
- A self-hosted GitHub runner must be dedicated, controlled, patched and
  restricted to manual, reviewed workflow dispatches on the main branch.
  Do not run arbitrary pull requests on a privileged public-repository runner.
- The label stackdeck-hyperv-qa must be assigned ONLY after physically
  validating that a new dedicated host contains no valuable virtual machines.

On the new computer, in Administrator PowerShell from a clean source checkout:

~~~powershell
.\scripts\windows\18-Host-Qualification-Intake.ps1 -RequireDisposableHost -Output .\host-intake.json
.\scripts\windows\17-Elevated-Release-Preflight.ps1 -Output .\full-host-preflight.json
.\scripts\windows\16-Ensure-Prebuilt-Runtime-Prereqs.ps1
~~~

Any blocked check stops qualification. Archive original reports, Windows build
and version, hardware type, VM inventory and file checksums.

## 2. Live runtime acceptance on the qualified host

After the read-only host intake passes and the operator approves test VM
creation, extract the independently verified RC5 LOCAL-GO ZIP. From a fresh
elevated Windows session, install an unsigned test MSI, documenting install logs,
exact source commit and hashes.

~~~powershell
msiexec.exe /i '.\installers\StackDeck_0.6.0_x64_en-US.msi' /L*v '.\msi-install.log'
.\qualification\08-Initialize-Installed-Runtime.ps1
.\qualification\11-Verify-Installed-Client.ps1 -SkipSignature -Output '.\installed-client-test.json'
.\qualification\04-Smoke-Test.ps1 -StackDeckExe '.\cli\stackdeck-cli.exe' -ComposeFile '.\examples\smoke\docker-compose.yml'
~~~

The runtime may download an immutable Debian image and create a new Hyper-V VM
only after checking that no VM or VM disk is already registered. Testing
provisioning mutates the *dedicated test workstation*, not the main development
machine. These commands must NOT run unattended on a valuable host.

The operator must separately record:
1. Debian QCOW2 download/checksum/conversion and VM disk type/size.
2. VM creation, UEFI boot, SSH, containerd/nerdctl/BuildKit health.
3. Single container create/start/log/stop/remove and OCI image pulls.
4. Compose up/status/down, test HTTP 200 and host port routing.
5. Container recovery after a controlled restart of the disposable VM
   and then after a controlled Windows test-host restart.
6. Verify cleanup leaves no test Compose services or leaked port rules.
7. A *real* approved prior-version -> RC5 upgrade with user data preserved.
8. Both MSI and NSIS clean install, repair, uninstall and desktop first-run.

A failure in one of these checks means the candidate is not qualified. Do not
synthesize a passing report or copy unrelated host evidence.

## 3. Trusted publisher certificate

The RND machine's only observed code-signing certificate is
"CN=PyStack Container Self-Signed" and must NOT be used for public release.
Have the intended legal publisher complete CA identity verification and obtain
a publicly trusted code-signing credential; use an approved hardware-backed
key service (HSM, hardware token, or compliant remote signing service), not an
unprotected locally generated PFX.

Microsoft Azure Artifact Signing offers cloud managed signing but as of October
2026 its Public Trust onboarding is restricted to verified organizations in
listed jurisdictions (and eligible individual developers in the US/Canada).
A Bangladesh-only legal publisher is not eligible for Azure Public Trust.
Private Trust is NOT a substitute for a publicly trusted Windows client.

The repository's current signed release workflow uses protected GitHub
environment values and PFX-based import. Do not load a non-exportable hardware
key into those PFX settings. If using a managed HSM service, adapt the protected
signing job to the provider's supported SignTool KSP/provider or official action
and verify a full end-to-end timestamped signature before permitting publish.
No private key material belongs in Git, issue text or build artifacts.

## 4. Fail-closed public release gate

Only after the dedicated host passes every live acceptance check:
- Preserve an independent SHA-256 qualified-host test report.
- Supply the exact qualified Git SHA and evidence digest to the protected
  GitHub windows-release environment.
- Require environment reviewers and a restricted signing identity.
- Build/sign CLI, Hive and desktop executables before packaging; sign both
  MSI and NSIS; verify embedded binary SHA-256 and authentic trust/timestamp.
- Recheck signatures on a clean Windows 11 computer.
- Publish release v0.6.0 only for the exact independently qualified commit.
  The RC5 annotation must not be represented as a signed production release.

**Current readiness: source and unsigned installer lifecycle qualified; host,
container integration, approved prior-version upgrade, CA-issued signing
and final public publication remain BLOCKED.**
