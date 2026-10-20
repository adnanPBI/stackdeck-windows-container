# StackDeck v0.6.0 — public launch and technical communication kit

**Publication status:** Official GitHub *unsigned evaluation prerelease* **published**; this is not a stable production release.
LinkedIn, Medium, DEV Community and other third-party posts are **drafts only**
until the author connects a permitted publishing account. Never describe
StackDeck v0.6.0 as a signed or stable/production-qualified Windows product.

Canonical launch URL:
https://github.com/adnanPBI/stackdeck-windows-container/releases/tag/preview-v0.6.0-c9607be36cc9

Exact tagged source: `c9607be36cc9acb14bd13020625b02a7a9838e9e`.
Successful independent CI:
https://github.com/adnanPBI/stackdeck-windows-container/actions/runs/39409784504
Technical qualification:
https://github.com/adnanPBI/stackdeck-windows-container/blob/release/v0.6.0-final-qualification/docs/STACKDECK_V060_PUBLIC_QUALIFICATION_20261020.md

## LinkedIn announcement (ready to publish when connected)

**StackDeck v0.6.0: a Windows Hyper-V container management preview**

I've published an **unsigned evaluation prerelease** of StackDeck, a native
Windows desktop and Rust CLI for container workloads running in a controlled
Debian Hyper-V guest. It uses containerd, nerdctl and BuildKit — without
requiring Docker Desktop or WSL2.

Working features validated on our test workstation include OCI container,
image, named-volume and network lifecycle; Dockerfile builds; Compose
operations with healthy Nginx and Windows HTTP 200; and a token-protected
local subset of the Docker Engine API.

Our locked Rust suite passed **124 tests (0 failed)**, and an independent
Windows GitHub Actions runner built and checked both MSI and NSIS
installers, embedded payloads, migration guard and support-log redaction.
The installed Hive passed repeated real Compose startup/teardown tests.

**Important limitations:** The preview is unsigned and experimental.
A new host-reboot/workload recovery, fresh clean-PC installer and historical
upgrade acceptance, trusted publisher signing and stable approval remain
outstanding. It is a technical evaluation, not a fully compatible Docker
Engine or certified production alternative.

Release, code and verification:
https://github.com/adnanPBI/stackdeck-windows-container/releases/tag/preview-v0.6.0-c9607be36cc9

#Rust #Windows11 #HyperV #Containers #BuildKit #OpenSource #DevTools

## Medium article draft

**Title:** Building StackDeck v0.6.0: a Windows Hyper-V container manager without Docker Desktop or WSL2

**Subtitle:** What the new unsigned developer preview actually supports, what was tested, and what's still blocking a stable release

### Why StackDeck exists

Windows developers sometimes want to run OCI containers using the Hyper-V
virtualization stack they already have, without keeping a separate Docker
Desktop or WSL2 runtime. StackDeck explores that approach with a Rust CLI
and a Tauri/React desktop interface that coordinate a controlled Debian
guest running containerd, nerdctl and BuildKit.

This is not an effort to replace every Docker feature. The architecture
deliberately separates host management, guest container operations,
Windows TCP port exposure and a small local Docker Engine API compatibility
surface. The API requires bearer-token authorization by default.

### What is working in the evaluation

On the authorized Windows 11 RND workstation, StackDeck exercised image,
network, named-volume and container lifecycle operations. We built a
Dockerfile with BuildKit, started Nginx through Compose, received
Windows HTTP 200 and confirmed project-owned cleanup afterward.

One particularly useful bug came from the teardown path. Nginx was
healthy, but containerd sometimes still reported an active task after
nerdctl acknowledged a stop. Removing the Compose network immediately
would be unsafe. The corrected Hive now checks state, uses bounded retries
against the exact project container, and leaves unrelated workloads and
networks untouched. The final candidate passed three complete Compose
cycles; the installed corrected Hive then passed the original smoke test
twice.

The locked Rust workspace reported 124 passed and zero failed.
Separately, a fresh GitHub Windows runner built both MSI and NSIS
installers, verified embedded payload hashes, exercised a simulated
historical-user NSIS migration guard and tested support-log redaction.

### Backup and host-restart discipline

The team retained the original approved Hyper-V VM and backups. All
13 encrypted off-device backup files passed SHA-256 verification, and
five approved VM-export components were authenticated and checked
through AES-GCM decryption in memory without restoring live disks.
A protected S4U post-boot verification task passed a dry run and correctly
reported that no real Windows reboot had occurred.

That final distinction matters. The workstation was actively running
Excel and browsers, and the remote agent did not have proven unattended
reconnection. We chose not to restart it and claim a recovery success
without observation.

### What's available — and what isn't

The official **unsigned evaluation prerelease** includes a Tauri
desktop binary, Rust Hive/CLI, Windows MSI and NSIS installers,
SHA-256 hashes and provenance artifacts. Both installers were produced
by independent GitHub Actions at exact tagged commit
`c9607be36cc9acb14bd13020625b02a7a9838e9e`.

The preview is not Authenticode-signed. It has not passed an independent
fresh-Windows installation/repair/upgrade/uninstall matrix or a real
unattended Windows host reboot with portproxy and workload recovery.
Nor does it claim complete Docker API parity.

For a stable release, the remaining work is concrete: safely qualify
the host restart and workload recovery; test fresh Windows installers
and the legacy upgrade path; independently escrow backup recovery
material; obtain trusted publisher signing and timestamp evidence; then
approve the exact release commit and its evidence digest.

**Explore the GitHub evaluation prerelease:**
https://github.com/adnanPBI/stackdeck-windows-container/releases/tag/preview-v0.6.0-c9607be36cc9

**Review the technical evidence:**
https://github.com/adnanPBI/stackdeck-windows-container/blob/release/v0.6.0-final-qualification/docs/STACKDECK_V060_PUBLIC_QUALIFICATION_20261020.md

### Note for evaluators

Use a disposable or well-backed-up Windows 11 Pro/Enterprise/Education
Hyper-V machine. Do not disable SmartScreen, antivirus or company
execution policy to bypass unsigned code warnings. Verify SHA-256
and review the source; unsigned checksums are integrity checks, not
cryptographic publisher authentication. Report errors through the
sanitized support bundle instructions in the repository.

## DEV Community (dev.to) article

**Suggested title:** Showcasing StackDeck v0.6.0 — Hyper-V containers on Windows, without WSL2

**Tags:** `windows`, `rust`, `devops`, `opensource`

StackDeck is a Rust CLI and Tauri desktop prototype for managing OCI
containers through an approved Debian Hyper-V guest. It uses containerd,
nerdctl and BuildKit and does not require Docker Desktop, WSL2 or a host
Docker Engine.

In the latest **unsigned experimental preview**, I tested image/volume/
network operations, Dockerfile builds, Compose health and teardown, HTTP
200 via Windows TCP exposure, and a bearer-protected local Docker API
subset. A containerd teardown race was fixed with project-scoped waits
and retries; the installed application passed two exact smoke test
cycles after three candidate cycles.

The Rust workspace passed 124 locked tests. An independent Windows
GitHub runner passed MSI/NSIS build, embedded payload checks, legacy
installer safety and log-redaction tests.

**Not a production release:** independent clean-client install and
real host-restart/workload recovery are still missing; installers are
unsigned and compatibility is intentionally limited.

Download and inspect the official prerelease:
https://github.com/adnanPBI/stackdeck-windows-container/releases/tag/preview-v0.6.0-c9607be36cc9

Technical results:
https://github.com/adnanPBI/stackdeck-windows-container/blob/release/v0.6.0-final-qualification/docs/STACKDECK_V060_PUBLIC_QUALIFICATION_20261020.md

## Hacker News / community technical thread

**Suggested title:** Show HN: StackDeck — experimental Windows Hyper-V OCI container manager

**Suggested text:**

I've released an unsigned v0.6.0 preview of StackDeck, a Rust CLI
and Tauri desktop that operate OCI containers in a controlled Debian
Hyper-V guest using containerd/nerdctl/BuildKit. No Docker Desktop or
WSL2 is required. I've documented the exact smoke/Compose/BuildKit,
volume/network, and limited token-protected Docker API tests, plus
the current failure boundaries. The latest independent Windows CI
passed and built MSI/NSIS. This is **not yet a stable or signed
product**; fresh-PC install, unattended Windows reboot and workload
recovery remain open. Technical feedback welcome.

Source and release:
https://github.com/adnanPBI/stackdeck-windows-container/releases/tag/preview-v0.6.0-c9607be36cc9

Before sharing in any community, read that community's rules on
self-promotion and disclose your role as the builder.

## Cross-posting checklist (manual or through authorized account)

- Publish **only** in an account the author controls or has explicit posting permission for.
- Keep the wording **unsigned evaluation preview** visible; never use “stable”, “production certified”, or “full Docker replacement”.
- Link to the *official GitHub prerelease*, not a separately hosted executable.
- Do not repost identical copies repeatedly or imply sponsorship or endorsement by Docker, Microsoft, Debian, DigiCert, or OpenAI.
- Verify the post is actually live and retain its URL before claiming publication.
- If a connector is not connected, treat this file as **ready-to-post draft content**, not a published post.
