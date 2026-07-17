# Offline GUI Example Container Proof - 2026-06-05

This proof was captured from `examples/offline_app_bundle/app` using the PyQt6 offline GUI bundle runner.

## Result

- Hyper-V backend health: healthy.
- Example compose validation: passed.
- Example services created: `web`, `api`, `worker`, and `cache`.
- Running containers: `pystack-app-web`, `pystack-app-api`, `pystack-app-worker`, and `pystack-app-cache`.
- Runtime image present: `busybox:1.36`.
- Web endpoint: `http://127.0.0.1:8080`.
- API endpoint: `http://127.0.0.1:8000`.
- Screenshot: `offline-gui-example-worker-2026-06-05.png`.

## Evidence

```text
Saved Hyper-V backend config: C:\Users\Administrator\.pystack_runner\hyperv.json
VM: pystack-linux
SSH: pystack@172.31.52.208:22
{
  "vm_name": "pystack-linux",
  "vm_state": "Running",
  "ssh_host": "172.31.52.208",
  "ssh": "reachable",
  "containerd": "active",
  "nerdctl": "nerdctl version 2.3.1",
  "namespace": "pystack",
  "ok": true,
  "namespace_ready": "yes"
}
web: hyperv container(s) started 97066a5c45a0
api: hyperv container(s) started aa4e02f9b3dd
worker: hyperv container(s) started d4209dc96979
cache: hyperv container(s) started 5820bf5fa4ba
Project: app
SERVICE              STATUS       PID      RESTART      CWD
web                  running      -        always       D:\python-hyperv-container-windows\examples\offline_app_bundle\app
api                  running      -        always       D:\python-hyperv-container-windows\examples\offline_app_bundle\app
worker               running      -        always       D:\python-hyperv-container-windows\examples\offline_app_bundle\app
cache                running      -        always       D:\python-hyperv-container-windows\examples\offline_app_bundle\app

Hyper-V containers:
5820bf5fa4ba    docker.io/library/busybox:1.36    "sh -c while true; d..."    4 seconds ago     Up                                  pystack-app-cache
d4209dc96979    docker.io/library/busybox:1.36    "sh -c while true; d..."    9 seconds ago     Up                                  pystack-app-worker
aa4e02f9b3dd    docker.io/library/busybox:1.36    "sh -c mkdir -p /api..."    14 seconds ago    Up        0.0.0.0:8000->8000/tcp    pystack-app-api
97066a5c45a0    docker.io/library/busybox:1.36    "sh -c mkdir -p /www..."    19 seconds ago    Up        0.0.0.0:8080->80/tcp      pystack-app-web
REPOSITORY     TAG       IMAGE ID        CREATED         PLATFORM       SIZE       BLOB SIZE
busybox        1.36      73aaf090f3d8    2 hours ago     linux/amd64    4.551MB    2.207MB
hello-world    latest    0e760fdfbc48    19 hours ago    linux/amd64    16.38kB    4.015kB

HTTP 8080: PyStack offline web placeholder
HTTP 8000: {"ok":true,"service":"api"}
```

## Note

The example compose now uses the small `busybox:1.36` placeholder image for all services so the default Start path can run from a preloaded/offline image instead of pulling larger public images from Docker Hub. Runtime image layers, VHDX disks, generated `.pyz` release artifacts, screenshots, and recordings are intentionally ignored and are not committed to GitHub unless placed under `docs/proofs` as an explicit lightweight proof.
