# StackDeck Hyper-V Runtime Completion Plan

  ## Summary

  Implement a breaking full rename from pystack/PyStack to stackdeck/StackDeck, fix the
  Hyper-V SSH readiness issue found during local deployment, then prove the runtime end
  to end on this Windows Hyper-V host: VM provisioning, SSH/bootstrap, containerd +
  nerdctl, Compose smoke, TCP portproxy, and Docker-compatible API subset.

  ## Key Changes

  - Rename product/runtime identity everywhere:
      - Binary target: replace pystack.exe with stackdeck.exe; keep hive.exe.
      - Rust crate/package names and imports: pystack-* to stackdeck-*.
      - Runtime identifiers: VM stackdeck-linux, VM root F:\StackDeck\VMs\stackdeck-
        linux, namespace stackdeck, container prefix stackdeck-, SMB/share/temp paths,
        generated passwords, diagnostics labels, GUI text, docs, scripts, and examples.

      - Config/state paths: move defaults from .pystack_runner and .pystack to StackDeck
        equivalents. No backward-compatible alias is required.

  - Update Windows scripts:
      - Rename PystackExe parameters to StackdeckExe.
      - Default VM root to F:\StackDeck\VMs\stackdeck-linux.
      - Build stackdeck.exe and hive.exe.
      - API scheduled task becomes StackDeck Local Docker API.
      - Token env var becomes STACKDECK_DOCKER_API_TOKEN.

  - Fix Hyper-V runtime readiness:
      - Change ssh_probe() so it returns true only when SSH exits successfully.
      - Ensure wait_for_ssh() waits through connection refused/timeouts.
      - Keep SSH noninteractive options.
      - Make bootstrap run only after verified SSH readiness.

  - Use reliable Ubuntu image flow:
      - Use Canonical generic jammy-server-cloudimg-amd64.img.
      - Require qemu-img unless a known-good preconverted NoCloud-compatible VHDX is
        supplied.

      - Avoid Azure VHD as the default path because it booted locally but did not expose
        SSH.

  - Docker-compatible API coverage:
      - Rebrand env/config names.
      - Keep bearer-token requirement.
      - Preserve supported subset: ping/version, container create/start/stop/remove/
        logs/inspect/list, images, volumes, networks.

      - Keep unsupported endpoints returning clean 404.

  ## Implementation Sequence

  1. Rebrand workspace metadata, crate package names, dependency names, Rust imports,
     binary target, scripts, docs, examples, GUI labels, and tests.

  2. Update default runtime constants: VM name, VM root, SSH user if desired as
     stackdeck, namespace, registry dir, state/log dirs, token env var, container/
     network/share/temp prefixes.

  3. Fix ssh_probe() and add tests proving failed SSH exits do not count as reachable.
  4. Update deployment scripts and docs to use stackdeck.exe, F:
     \StackDeck\VMs\stackdeck-linux, and STACKDECK_DOCKER_API_TOKEN.

  5. Remove or clearly ignore old local failed test VM/resources named pystack-linux
     before final proof run.

  6. Run source gates:
      - cargo fmt --check
      - scoped core tests
      - release build for stackdeck.exe
      - release build for hive.exe

  7. Run elevated runtime proof:
      - prerequisite check
      - configure Hyper-V
      - init VM from generic Ubuntu image
      - verify SSH, containerd, nerdctl
      - run Compose smoke
      - verify http://127.0.0.1:18080

  8. Run API proof:
      - install/start API task
      - test auth failure and success
      - test supported container/image/volume/network endpoints
      - confirm unsupported endpoint returns 404.

  ## Test Plan

  - Unit tests:
      - renamed constants/defaults serialize correctly.
      - ssh_probe() returns false on nonzero SSH exit.
      - API auth uses STACKDECK_DOCKER_API_TOKEN.
      - container names/networks use stackdeck- prefix.
      - Compose conversion still preserves ports, env, healthchecks, depends_on,
        volumes.

  - Build tests:
      - no pystack binary target remains.
      - target\release\stackdeck.exe exists.
      - target\release\hive.exe exists.

  - Runtime acceptance:
      - stackdeck hyperv health reports VM running, SSH reachable, containerd active,
        nerdctl available.

      - stackdeck compose up -f examples\smoke\docker-compose.yml --backend hyperv -d
      - browser/curl reaches http://127.0.0.1:18080.
      - stackdeck compose status/logs/down works.
      - API shim works with bearer token and rejects missing token.

  ## Assumptions

  - This is a breaking rename: no pystack.exe, no old env var compatibility, no old
    config-dir compatibility.

  - hive.exe remains as the sidecar binary because it does not contain the pystack name.
  - Final proof must run locally with Administrator privileges, Hyper-V enabled, and
    qemu-img available or installed.

  - UDP publishing remains unsupported; Docker API remains a supported-workflow subset,
    not full Docker Engine parity.