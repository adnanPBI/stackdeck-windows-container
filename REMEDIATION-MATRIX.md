# Remediation matrix

## Runtime and security

| Finding | Disposition | Implementation / verification |
|---|---|---|
| Elevated PowerShell command injection | Implemented | Compose port strings are parsed to typed IPv4/u16/protocol values. `netsh.exe` receives fixed argument vectors. Malicious port regression cases are included. |
| Unrelated-process termination | Implemented | Stop/force-start require the persisted creation-time, executable, and command-line identity. Identity is re-read after the graceful-delay window and immediately before `taskkill /F`; a changed identity retains state and fails closed. PID liveness uses exact Windows process handles. |
| Destructive or stale service starts | Implemented | Containers carry a deterministic configuration-hash label. An unchanged stopped container starts in place; changed image/ports/environment/mount configuration triggers replacement. Explicit build and drift replacement stop and rename the old container to a rollback name, launch and health-check the replacement, then delete the backup only after success. Failed launch restores the prior writable layer/container. |
| Lost SSH key during Hyper-V initialization | Implemented | IP discovery updates the already-mutated configuration instead of cloning stale state. |
| Missing BuildKit | Implemented | Bootstrap installs the checksum-verified `nerdctl-full` distribution, enables `buildkitd`, and verifies a worker. Health reports BuildKit readiness. |
| Registry mirror permissions/removal | Implemented | Registry/endpoints are validated, privileged writes use `sudo`, directories and `hosts.toml` receive explicit modes, and containerd restart failures propagate. Removal deletes the guest `certs.d/<registry>` directory and restarts containerd before committing the local JSON removal. |
| Guest-shell SMB injection / password exposure | Implemented | Windows hostnames and every path/name operand are validated or shell-quoted. Passwords are transferred in a temporary credential file, not interpolated into the guest command. Deployment scripts require `SecureString`. |
| Persistent state loss / cross-process races | Implemented | Corrupt API/registry JSON is preserved and causes an error. Writes are atomic and read-modify-write operations use process and cross-process lock guards backed by stable OS file locks; no age-based lock deletion or path unlink occurs. SQLite migration is transactional, and service upsert is one atomic `INSERT ... ON CONFLICT` merge rather than a stale read followed by write. |
| Timed command pipe deadlock | Implemented | PowerShell and captured SSH stdout/stderr are drained concurrently while the child runs. Captured diagnostic tails are bounded, and timeout termination joins both readers. |
| Download digest only enforced in script | Implemented | The Rust backend requires a digest, hashes the downloaded `.partial` bytes, deletes mismatches, and promotes only verified content. |

## Docker API

| Finding | Disposition | Implementation / verification |
|---|---|---|
| UDP treated as TCP | Safe, explicit compatibility gate | Protocol and `HostIp` survive normalization/inspection. The Docker API now rejects a UDP host publish at create time, before any state or runtime mutation, because Windows `v4tov4` cannot proxy UDP. The released compatibility contract explicitly supports TCP host publishing only. |
| Discarded `HostIp` | Implemented | Windows listen address is retained and typed. Guest publishing remains VM-reachable while Windows portproxy enforces the requested host address. |
| Racy ephemeral allocation | Implemented | Allocation occurs inside the cross-process state transaction, adds every selection to the request reservation set, and retains bound host listeners from create through guest start. Listeners are released only immediately before portproxy activation; API restart restores reservations for not-yet-started containers. |
| Hardcoded TCP inspect | Implemented | Inspect keys use the persisted protocol. |
| Docker log framing | Implemented | Non-TTY logs use Docker's 8-byte multiplex header; TTY logs remain raw. |
| Pull progress | Implemented | Separate stdout/stderr line decoders preserve records split across read chunks. A bounded synchronous pump feeds the bounded Tokio channel, so backpressure reaches the SSH child; a disconnected HTTP client cancels the remote command. Partial-line storage is capped. |
| Local daemon authentication | Implemented, fail-closed | Bearer authentication is mandatory by default on loopback and remote listeners. Missing token configuration prevents server startup, and browser-originated blind requests cannot supply the bearer header. The unauthenticated mode requires the explicit compatibility-only `STACKDECK_DOCKER_API_REQUIRE_TOKEN=0` override. A Windows named-pipe transport remains a future compatibility/defense-in-depth improvement, not the default trust boundary. |
| Crash-consistent API start | Implemented | API state commits `runtime_pending=true` before guest mutation. Successful start resolves and persists the real runtime ID; failures retain a conservative reconciliation record or roll back newly introduced runtime state. Removal trusts neither `runtime_id: None` nor stale output and reconciles the deterministic runtime name. |

## Compose and stack normalization

| Area | Disposition |
|---|---|
| YAML anchors / merge keys | Expanded before normalization with regression coverage. |
| Interpolation | Supports unset/empty default, required, alternate-value, simple `$VAR`, and `$$` escape forms. |
| Entrypoint / command | Preserved separately and applied in native and Hyper-V execution order. |
| Long-form ports / anonymous volumes | Normalized; target-only ports are no longer accidentally published; anonymous targets are retained. |
| Secrets / configs | File, content, and environment sources are materialized with validated target/mode/uid/gid and mounted read-only on Hyper-V. Unsupported external resources fail closed. |
| Networks | Named project networks and service DNS aliases are created. Unsupported drivers, driver options, static addresses, explicit aliases, and priorities fail validation rather than being ignored. |
| Replicas | Replica services expand to distinct runtime services. Fixed published ports and `container_name` with scale fail validation because this runtime has no ingress load balancer. |
| Healthchecks / env_file / restart / logging | Preserved and mapped. Restart supervision has bounded exponential backoff. |
| Build configuration / `.dockerignore` | Context, Dockerfile, target, args, cache-from, and cache-to are mapped. Unknown build options fail closed. BuildKit receives the original `.dockerignore`, including ordered negation rules. |

## Runtime policy and recovery

| Finding | Disposition |
|---|---|
| Restart limits/backoff | Implemented in both supervise and watch paths, capped at five minutes between attempts. |
| DHCP recovery | VM state and IP are rediscovered; a changed address is persisted before retry. |
| Mixed native/Hyper-V status | Both backends are returned; JSON groups them by backend. |
| Offline portproxy cleanup | Host entries are removed even if SSH/VM is offline. |
| Transactional migrations | Legacy SQLite migration uses one transaction and state reads no longer swallow DB errors. |
| Ignored CLI flags | Project volume/network naming is honored; unsupported network drivers are rejected. |
| Unbounded SSH/build/pull/health operations | Implemented | PowerShell, SSH, SCP, interactive SSH input, build, and pull commands use hard local deadlines; build receives an explicitly longer bounded deadline. |

## Desktop and supply chain

| Finding | Disposition |
|---|---|
| WebView2 clean-machine launch | Tauri bundles the offline WebView2 installer. Must be proven on a clean Windows VM. |
| Rust 1.75 vs locked graph | Workspace, toolchain file, prereq script, and CI pin Rust 1.88. |
| Unsigned sidecars/installers | Signing script signs and verifies CLI, sidecars, MSI, and NSIS artifacts using SHA-256 and RFC3161 timestamping. |
| Authenticode workflow | Tag/manual Windows workflow uses a protected signing environment, imports an operator certificate, signs CLI payloads, builds and verifies the production desktop PE, signs the embedded desktop/sidecar payloads before bundling, signs MSI/NSIS, and re-verifies all six payloads. |
| Artifact verification | CI actions are commit-pinned. The workflow emits and rechecks `SHA256SUMS.txt`, writes source/tool/artifact `provenance.json`, creates a GitHub Sigstore/SLSA provenance attestation, uploads the signed workflow artifact, and publishes only the canonical signed MSI/NSIS plus evidence on version-matched tags. |

## Release decision

This patch materially covers every reported workstream, but it is not evidence
of production readiness by itself. Release must remain blocked until:

1. Rust formatting/tests/build pass on Rust 1.88 for Windows MSVC.
2. A clean Windows machine completes WebView2 installation and launches both
   MSI and NSIS builds.
3. Hyper-V initialization, DHCP recovery, BuildKit build, restart recovery,
   SMB mounts, state corruption, and PID-reuse scenarios pass end to end.
4. Docker pull streaming and registry-authenticated private pulls pass against
   a live registry, including client disconnect and timeout behavior.
5. TCP-only host publishing is documented in the released compatibility
   contract and API rejection coverage remains green until a UDP relay exists.
6. Authenticode verification and provenance attestation pass using the actual
   release certificate.
