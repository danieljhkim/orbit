## Context
Linux CLI agents previously ran with the worker account's ambient filesystem rights. The real alternatives were a Bubblewrap mount-namespace boundary, a Landlock allowlist layer, or continued delegation to provider-native sandboxes; Bubblewrap closes the highest-value write gap at the existing executor wrapper seam without turning Orbit into a container runtime.

## Decision
Shipped Linux agent executors use the concrete `linux-bwrap` backend. Orbit resolves only `/usr/bin/bwrap`, capability-probes the namespaces and mounts it needs, mounts the host root read-only, rebinds canonical policy and runtime roots writable in rule order, retains the host network namespace, and fails closed unless `allow_fallback` explicitly permits bare execution. This backend claims `write_enforced` and `read_delegated`; read-policy parity remains deferred.

## Consequences
- Linux CLI agents gain kernel-backed write confinement without changing the generic `run_process + NoSandbox` contract or the macOS backend.
- Non-subtree deny globs are snapshot-expanded; managed single-writer worktrees receive a post-run new-match check, while direct overlapping invocations fail closed.
- Provider-native sandbox flags are neutralized only after the outer wrapper passes its capability probe, so bare fallback retains the provider boundary.
- Cost: Bubblewrap availability depends on `/usr/bin/bwrap` plus host user-namespace policy, and write confinement deliberately leaves the broad host read surface and provider network access delegated for a later decision.