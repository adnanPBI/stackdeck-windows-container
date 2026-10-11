# Production MVP architecture

## Runtime diagram

```text
StackDeck UI / stackdeck CLI
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
Windows 11 Pro, Enterprise, or Education x86-64 with Hyper-V (Windows 10 only when separately qualified with -AllowWindows10)
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
| 2 | Build | release `stackdeck-cli.exe` / `stackdeck-hive.exe` |
| 3 | Hyper-V config | `%SystemDrive%\StackDeck\VMs\stackdeck-linux`, 10 GB disk, memory, CPU, switch |
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
