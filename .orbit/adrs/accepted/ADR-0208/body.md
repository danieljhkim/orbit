## Context

Routines run on two hosts (dk-mac, dk-server-1) with different availability profiles. Definitions must converge across hosts; scheduler runtime state (last fires, pauses, locks) could either be synced between hosts or kept local. Syncing state would let either machine answer "did the nightly fire on the other box?" but requires a scheduler network protocol between hosts that only expose 22/443 to each other.

## Decision

Routine YAML definitions live in routine-source workspaces and converge via git like any other versioned definition. All scheduler state — fires (with idempotency keys), host-local pauses, and the sweep advisory lock — lives in a host-local SQLite routine store, gitignored and never synced. No scheduler network protocol exists in v1.

## Consequences

- Two hosts converge on definitions through a normal `git pull`; no new sync mechanism to build, secure, or debug.
- State stays consistent with the run history it references, which is also host-local.
- Cost: cross-host observability requires asking each host — there is no single pane of glass, and a definition edit is only as fresh on the other host as its last `git pull`.