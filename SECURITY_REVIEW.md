# Security Review

This review documents the current security, offline GUI, and secret-handling posture for StackDeck (formerly PyStack Runner).

## Desktop GUI (Tauri)

- StackDeck now uses a fully offline Tauri desktop application.
- It no longer exposes a Python-based HTTP web GUI on `127.0.0.1`.
- The GUI operates using local IPC via Tauri commands, reading from the local project registry and directly executing the Rust CLI sidecar (`stackdeck-hive.exe`).
- There is no longer a bearer token (`PYSTACK_GUI_TOKEN`) required, as the interface is inherently local and bound to the host desktop session.
- Remote access to the StackDeck GUI is not supported by design.

## Live SMB Mounts

- The Hyper-V backend creates and utilizes live Windows SMB mounts to expose repository paths into the Linux VM for `containerd`/`nerdctl` bind mounts.
- When `stackdeck-hive.exe hyperv share add` and `share mount` are used, StackDeck automatically stages Windows credentials to authorize the Linux VM to access the shared folder.
- Ensure that you only run trusted containers and images. Processes running as `root` inside the Linux VM have access to the mounted Windows SMB shares matching their bind-mount configuration.
- Modifying files inside a container bind mount will directly modify files on the Windows host.

## Secret handling

- Secret-like keys containing `PASS`, `PASSWORD`, `SECRET`, `TOKEN`, `KEY`, `PRIVATE`, or `CREDENTIAL` are redacted in diagnostics output.
- Log rendering masks common token/password/key patterns and known local secret values from the environment and Hyper-V config.

Remaining risk:

- Redaction is best-effort pattern matching. It can miss application-specific formats, short secrets, encoded secrets, multiline private keys, or secrets embedded in opaque blobs.
- Diagnostics still include topology and operational metadata such as paths, usernames, hostnames, ports, service names, image names, and health errors.
- Running services receive configured environment variables; StackDeck cannot prevent a service from logging its own secrets.

## Docker API Shim

If the Docker API shim is used, keep it bound to localhost for workstation use:

```powershell
stackdeck-hive.exe daemon serve --host 127.0.0.1 --port 23750
```

If remote access is required, place the listener behind HTTPS, network allowlisting, and an identity-aware reverse proxy. Do not rely on the Docker API shim alone as an internet-facing authorization boundary.

## Diagnostics sharing

Generate diagnostics with:

```powershell
stackdeck-hive.exe --config stack.json diagnostics
```

Review the zip or text report before sending it outside the host. The diagnostics command intentionally redacts config and log excerpts, but the operator remains responsible for approving what leaves the machine.
