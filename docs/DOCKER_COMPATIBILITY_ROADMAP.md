# StackDeck Docker-Like Compatibility Roadmap

Scope: local Windows container development on a single Hyper-V Linux VM. Kubernetes,
Swarm, clustering, extensions, and remote orchestration are intentionally out of scope.

## Stage 1 - Product Identity And State Isolation

- Ship `stackdeck.exe` as the primary CLI.
- Keep old crate names temporarily, but remove user-visible PyStack runtime defaults.
- Use `%USERPROFILE%\.stackdeck_runner` for project/API/Hyper-V state.
- Use `.stackdeck` for project-local logs and state.
- Use `stackdeck` as the containerd namespace and `stackdeck-*` runtime name prefix.
- Use `STACKDECK_DOCKER_API_*` environment variables.

## Stage 2 - Docker-Like CLI Surface

- Add top-level Docker-like aliases for high-frequency local development:
  `ps`, `images`, `pull`, `rmi`, `exec`, `logs`, `volume`, `network`, and `compose`.
- Keep advanced VM operations under `hyperv`.
- Add compatibility tests that validate command parsing and generated `nerdctl` commands.
- Produce a command matrix documenting supported, partially supported, and unsupported
  Docker CLI flags.

## Stage 3 - Docker Engine API Coverage

- Expand the local loopback API shim for common developer tools:
  container inspect/list/create/start/stop/remove/logs, image list/pull/inspect/remove,
  volume list/create/inspect/remove, network list/create/inspect/remove.
- Add explicit unsupported-route responses with Docker-shaped error bodies.
- Add route tests for every supported API path and auth behavior.

## Stage 4 - Image, Build, Registry, And Auth UX

- Harden `nerdctl build` support for Compose build contexts, Dockerfiles, target stages,
  and build args.
- Add `stackdeck login/logout` wrappers around `nerdctl login/logout`.
- Store registry auth only inside the VM/runtime context, not in host Docker config.
- Document known differences from Docker buildx and BuildKit.

## Stage 5 - Local Desktop Parity

- Manage images, volumes, networks, container logs, and runtime diagnostics in the GUI.
- Add clear empty states so fresh installs do not show stale projects.
- Add first-run runtime setup status and actionable remediation messages.

## Stage 6 - VM And Bootstrap Hardening

- Make admin/elevation checks explicit before privileged operations.
- Fix SSH readiness to require successful command exit and expected probe output.
- Prefer prebuilt VHDX or verified cloud image conversion with strong logs.
- Add bootstrap phases that are idempotent and resumable.
- Add clean-machine proof: install, choose VM drive, initialize VM, bootstrap runtime,
  run Compose smoke workload, and use the API shim.

## Stage 7 - MSI Installer Parity

- Keep NSIS as the richer interactive installer path.
- Add a custom WiX authoring layer only when MSI drive-selection UI and custom actions
  are required by distribution constraints.
- Always support verbose MSI logging with `msiexec /l*v`.
