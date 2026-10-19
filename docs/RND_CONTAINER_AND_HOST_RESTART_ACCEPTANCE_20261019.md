# StackDeck 0.6.0 — RND container features and unattended restart evidence

**Scope:** RND Windows 11, only the existing approved `stackdeck-linux` VM, GUID `47F1AD41-9621-4513-AA01-1117E8E47B13`. No creation of `pystack-linux`, VM disk replacement, source runtime-lock changes, or deletion of existing recovery backups.

**Release classification: local installed-client functional qualification only. This is NOT trusted signed stable production GO.** The new S4U/Job Object startup support scripts are RND-specific and are **not** part of MSI/NSIS first-run provisioning for unrelated machines. Independent clean-client and actual host reboot remain open.

## What was directly tested on the installed v0.6.0

| Tested behavior | Evidence on RND | Status |
|---|---|---|
| Existing pinned image listing, new named volume and bridge network, inspect/list | `target/release-evidence/20261019-image-volume-network-live-v4.json` | PASS |
| Volume data survived deletion and recreation of its container | Same report | PASS |
| Scoped removal of only test volume/network/container, VM and config preserved | Same report | PASS |
| Dockerfile via Compose/BuildKit, tagged image, built container and unique HTTP 200 content, exact-resource cleanup | `target/release-evidence/20261019-buildkit-dockerfile-live.json` | PASS |
| Docker Engine REST API subset: bearer auth, negative auth, versioned ping, inspect/list, container and resource lifecycles | `target/release-evidence/20261019-docker-api-compat-live.json` | PASS |
| Separate S4U startup service, Hyper-V health, authenticated API 200, unauthenticated API 401 | `target/release-evidence/20261019-unattended-host-recovery-s4u-install.json` | PASS |
| Controlled task STOP **releases its API port**; START restores the listener and bearer authentication | `target/release-evidence/20261019-headless-api-jobguard-restart.json` | PASS |
| Read-only postboot verifier registered for the next **actual** host boot | `target/release-evidence/20261019-postboot-evidence-task-armed.json` | ARMED |
| S4U verifier dry-run distinguishes current-boot health from real reboot evidence | `target/release-evidence/20261019-postboot-dryrun-qualification.json` | PASS (dry run only) |
| **Actual Windows host power-cycle/reboot**, Windows login-independent recovery, application HTTP over recovered portproxy | No real reboot after baseline timestamp | **NOT QUALIFIED** |

The full Linux container engine is **not** the full Docker Engine: the API shim does not promise Buildx, Swarm, Kubernetes, arbitrary Docker plugins, UDP host forwarding or complete Docker SDK/CLI compatibility. Host network requests were explicitly rejected with HTTP 400. Network support tested was the default bridge.

**Destructive operations excluded:** no global image/volume/network prune, no unrelated container stop, no VM creation or deletion, no VM disk deletion, no wiping recovery backups.

## Current RND unattended service

Task: `StackDeck-v060-Unattended-DockerAPI`

- `AtStartup` trigger, `RND\NHTML`, S4U, highest privileges. It runs without an interactive desktop sign-in.
- Runner: `C:\ProgramData\StackDeck\ops\Unattended-HostRecovery.ps1`.
- Bearer token: `C:\ProgramData\StackDeck\config\docker-api-token.txt`. **Never copy its contents into tickets, terminal transcripts, or support ZIPs.**
- API: `http://127.0.0.1:23750`, **bearer authenticated only**. Exposing this listener to the LAN is not approved.
- Checks the exact VM name/GUID and approved installed Hive SHA-256. Waits for authenticated SSH, containerd, nerdctl and BuildKit.
- If the approved VM receives a changed unambiguous private IPv4 address, it uses the StackDeck CLI's guarded `hyperv discover-ip` flow for the machine-scoped guest SSH target.
- The daemon child is attached to a Windows **kill-on-job-close** Job Object. The controlled stop/start test proves it does not leave an orphan port listener.
- Previous login-only task `StackDeck Local Docker API` remains present but disabled. Its exported rollback definition and original launcher are preserved in protected `C:\ProgramData\StackDeck\ops`.

The first attempted SYSTEM task failed because Windows OpenSSH rejected the user-owned private SSH key. No private SSH key was copied or re-owned: the S4U task retains the original authorized identity. This distinction is essential for support and security reviews.

## Actual host-restart acceptance gate

The RND console was **actively used**, with open Excel and browser processes, when the verifier was armed. The remote desktop agent had no independently verified service auto-start/reconnection configuration. Therefore **Windows was not restarted**; unsaved user documents and remote access were not put at risk.

Task armed for next actual startup: `StackDeck-v060-PostBoot-Readiness`.

1. Save and close interactive work before an authorized maintenance-window restart. Preserve the verified Hyper-V export and off-device backup. Confirm the approved VM GUID and availability of an independent reconnect path.
2. Do not force power off the Hyper-V VM or terminate unapproved processes. Use normal Windows Restart only after these prerequisites are met.
3. The S4U verifier records a fresh JSON report under `target/release-evidence/20261019-postboot-actual-<unique>.json`. It compares the new Windows `LastBootUpTime` against the pre-reboot baseline and refuses to pass without a **new boot**.
4. It checks the exact approved VM state, guest SSH/containerd/BuildKit, service task S4U startup trigger, one API listener, HTTP 200 with the protected token, and HTTP 401 without the token.
5. Independently check any **explicitly approved production Compose workload's** restart policy, named volume contents and Windows-published HTTP/TCP service after boot. The RND postboot verifier cannot substitute for this. Its HTTP service baseline was not healthy when armed, so this additional portproxy/application qualification **is still required**.
6. Archive the new report SHA-256, task history, updated guest IP and sanitized support bundle, with explicit reviewer approval. Only then mark the host restart gate PASS.

**Portproxy caution:** a guest DHCP address change may leave preexisting Windows `netsh interface portproxy` connect targets stale. The S4U recovery launcher refreshes StackDeck's SSH IP, but **does not certify all preexisting Compose portproxy mappings**. Do not rewrite unrelated portproxy rules or infer working Windows HTTP from working SSH. A production-grade per-workload recovery policy must validate and reconcile only explicitly owned mappings.

## Safe operator usage

With PowerShell **elevated**, check service health and current task:

```powershell
Get-ScheduledTask -TaskName 'StackDeck-v060-Unattended-DockerAPI'
& 'C:\Program Files\StackDeck\stackdeck-hive.exe' hyperv health
```

Re-test the controlled headless API task stop/start, which temporarily interrupts only the local Docker API listener:

```powershell
.\scripts\windows\29-Test-Headless-ServiceRestart.ps1 -Output 'D:\My-Evidence\headless-qa.json'
```

The QA refuses to overwrite an existing report and refuses to manipulate another VM. Avoid rerunning concurrently with application management.

Scripts `24`, `25`, `26`, `27` and `29` contain **RND-specific release/VM assertions**: do not run them on a new client or clean machine as a generic installer. Script `25` is for first-time guarded RND setup and intentionally refuses silent replacement of an existing registered task. The guarded update/rollback and independent evidence were performed on RND; never modify `main` or the runtime lock as part of this workstation-only repair.

## Remaining production GO limitations

- Actual Windows restart and subsequent unattended recovery **not executed**, including dynamic DHCP portproxy and real workload recovery.
- Independent clean-user/fresh-device MSI+NSIS install and legacy upgrade.
- Published stable release binaries with authorized, trusted, timestamped Authenticode signatures.
- Independent reviewer authorization of the exact source commit and sanitized evidence digest.

**Conclusion:** Image/volume/network lifecycle, Dockerfile/BuildKit and the supported Docker API subset pass locally. S4U startup and controlled task lifecycle now pass. Actual Windows reboot/application recovery is pending and must not be advertised as passed.
