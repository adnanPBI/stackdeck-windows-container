# Production MVP architecture

## Runtime diagram

```text
StackDeck UI / pystack CLI
        |
        | local commands / optional localhost API
        v
Rust control plane crates
        |
        | PowerShell Hyper-V + SSH
        v
Hyper-V Linux VM
        |
        | containerd + nerdctl
        v
OCI containers
```

## Hard independence rule

The runtime must not invoke or require:

```text
wsl.exe
Docker Desktop
host docker.exe
host dockerd
host docker.sock
```

## Accepted dependencies

```text
Windows 10/11 Pro, Enterprise, or Education with Hyper-V
Administrator PowerShell for setup
Hyper-V PowerShell module
OpenSSH client
qemu-img or preconverted VHDX
oscdimg/genisoimage
SMB/CIFS
netsh portproxy for TCP
containerd + nerdctl inside VM
```

## Phase map

| Phase | Goal | Deployable output |
|---:|---|---|
| 1 | Host preflight | `00-Ensure-Prereqs.ps1` |
| 2 | Build | release `pystack.exe` / `hive.exe` |
| 3 | Hyper-V config | D:\ VM root, 10 GB disk, memory, CPU, switch |
| 4 | VM init | verified image, cloud-init, VM, SSH, bootstrap |
| 5 | Runtime smoke | nginx container on localhost TCP |
| 6 | API task | optional localhost Docker API shim |
| 7 | Diagnostics | redacted bundle |

## State machine target

```text
Absent
ImageDownloading
ImageVerified
VhdPrepared
SeedPrepared
VmRegistered
VmBooting
SshReady
RuntimeBootstrapping
RuntimeReady
Operational
Degraded
NeedsUserAction
```

The current MVP scripts prepare the local deployment path. The long-term production daemon should persist these transitions in a durable state DB and reconcile drift after crashes/reboots.
