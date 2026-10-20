# StackDeck v0.6.0 — technical launch communications (DRAFT)

**Hold broader publication until the GitHub evaluation pre-release actually exists
and its tests and links have been checked.** Do not say "production ready",
"trusted signed", "security audited", "Docker-complete", "zero bugs", or
"supported on all Windows systems". Explain that this is a Windows 11 Hyper-V
evaluation build with unsigned installers.

Canonical source: https://github.com/adnanPBI/stackdeck-windows-container

Preview downloads, only when verified live:
https://github.com/adnanPBI/stackdeck-windows-container/releases

## LinkedIn (short)

**StackDeck v0.6.0 — Windows Hyper-V container orchestration, evaluation preview**

I'm sharing an experimental StackDeck build for developers exploring a
containerd/nerdctl + BuildKit workflow on Windows 11 Pro/Enterprise/Education
with Hyper-V, without depending on WSL or Docker Desktop.

The project pairs a Rust CLI and Tauri desktop with an isolated Debian
Hyper-V guest. Tested workflows include authenticated SSH, OCI containers,
Compose, Windows HTTP port exposure and cleanup. The build includes a
private-data-aware diagnostic ZIP collector and explicit installer migration
safeguards.

**Important:** This is an **unsigned evaluation preview**, not a stable
production installer. A dedicated backed-up test computer is required;
independent clean-client and host restart tests remain open. Windows security
controls may block unsigned code.

Source and evidence: https://github.com/adnanPBI/stackdeck-windows-container

#Rust #Windows11 #HyperV #Containers #OpenSource #DevTools

## Medium / DEV.to / Hashnode (article intro)

# Building a container desktop for Windows with Rust, Tauri and Hyper-V

StackDeck explores an alternative architecture to a Docker Desktop/WSL workflow:
a Windows desktop UI coordinates a Debian Hyper-V guest running containerd,
nerdctl and BuildKit. The host-side control plane is written in Rust with a
Tauri desktop front end.

This technical writeup covers the system boundaries, immutable runtime image
checks, fail-closed Hyper-V disk safeguards, guest network and SSH recovery,
Compose and port forwarding, installer transitions from per-user to machine-wide,
and the differences between an unsigned evaluation package and a trusted
Windows production release.

The v0.6.0 evaluation build has 121 passing locked Rust workspace tests on the
RND qualification host; MSI and NSIS archives were generated and their embedded
payloads verified by SHA-256. Production publisher trust, an independent
Windows 11 client matrix and complete host restart tests remain open.

**Read the code and release limits:** https://github.com/adnanPBI/stackdeck-windows-container

## Show HN / technical forums (once preview checks finish)

Show HN: StackDeck – Rust/Tauri interface for containerd in a Windows Hyper-V guest

StackDeck is a Windows 11 (Pro/Enterprise/Education) experiment: a Tauri +
Rust frontend controlling a Debian Hyper-V guest with containerd, nerdctl and
BuildKit. No WSL or Docker Desktop dependency. I would appreciate feedback
on safe VM provisioning, networking recovery and installer migration.

The preview installers are **unsigned**, so use a dedicated backed-up test
machine and follow your Windows security policy; stable release qualification
is still underway. Repo: https://github.com/adnanPBI/stackdeck-windows-container

## Publication checklist

- Confirm the exact GitHub preview tag and source SHA resolve to the reviewed
  commit and the release is explicitly marked **Pre-release**.
- Confirm both downloads, their SHA-256 manifest and the provenance file exist
  and match the CI build.
- Review the source license and dependency licenses, provide screenshots or a
  short OBS-recorded demo **only if captured from the real installed UI**.
- Never post backups, credentials, customer data or private log ZIPs.
- Add a link to `docs/SUPPORT_AND_TROUBLESHOOTING.md` and warn about unsigned
  Windows security prompts.
- Publish communications only after a human has checked the installed preview
  on a dedicated host and is willing to field bug reports.
