---
type: runbook
summary: Install and verify the Bubblewrap host prerequisite that Orbit's Linux sandbox fails closed without.
tags: [operations, sandbox, linux, bubblewrap, apparmor]
paths: ["crates/orbit-exec/src/**"]
related_features: [policy-sandbox, executors]
related_artifacts: []
last_validated: 2026-09-03
---

# Prepare a Linux Host for Sandboxed Dispatch

Use this runbook after `orbit init` on a Linux host, before dispatching any agent, or when a
run fails with `bwrap: setting up uid map: Permission denied`.

## Why the probe fails closed

`orbit init` persists the host-appropriate sandbox into the shipped executor artifacts. On
Linux that value is `linux-bwrap`, which resolves the trusted wrapper at `/usr/bin/bwrap` and
fails closed if its namespace-and-mount capability probe cannot run. Ubuntu 24.04 (Noble) also
enables AppArmor restrictions on unprivileged user namespaces; without the distro's narrow
Bubblewrap profile the probe fails with the UID-map error above.

The Linux boundary enforces writes from the resolved policy. It leaves host filesystem reads
and host network access available, so it does not provide worktree-only reads or policy-gated
network egress. See [policy-sandbox](../design/policy-sandbox/) for the design.

## Install and verify on Ubuntu 24.04

The package ships `bwrap-userns-restrict` under `/usr/share/apparmor/extra-profiles/`. Copy
that narrow profile into `/etc/apparmor.d/`, load it, confirm AppArmor knows it, then run the
same capability shape Orbit probes:

```bash
sudo apt-get update
sudo apt-get install --yes bubblewrap apparmor-profiles
test -x /usr/bin/bwrap
test -f /usr/share/apparmor/extra-profiles/bwrap-userns-restrict
sudo install -m 0644 \
  /usr/share/apparmor/extra-profiles/bwrap-userns-restrict \
  /etc/apparmor.d/bwrap-userns-restrict
test -f /etc/apparmor.d/bwrap-userns-restrict
sudo apparmor_parser -r /etc/apparmor.d/bwrap-userns-restrict
grep -Fq 'bwrap-userns-restrict' /sys/kernel/security/apparmor/profiles

/usr/bin/bwrap \
  --die-with-parent \
  --new-session \
  --unshare-all \
  --share-net \
  --ro-bind / / \
  -- /bin/true
```

The final command must exit successfully.

## If the probe still fails

Do not disable `kernel.apparmor_restrict_unprivileged_userns` globally and do not enable
`allow_fallback`; both weaken or bypass the fail-closed boundary. Re-check the packaged
profile path and the `apparmor_parser` output, then rerun the probe.

Other distributions need the same two conditions: an executable `/usr/bin/bwrap` and
permission for an unprivileged user to create user namespaces and mounts. Non-Linux hosts
are unaffected: macOS uses `sandbox-exec`, and platforms without a shipped OS-level backend
rely on in-process filesystem guards only.
