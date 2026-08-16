## Context

[ADR-0350] commits to the SSH tunnel as *reusable infrastructure*: "anything that
needs to reach the remote machine rides it rather than opening a second
mechanism." At the point [ORB-10710] added the second consumer, that reuse was not
structurally possible.

The only attach-or-spawn tunnel in the tree lived in
`orbit-dashboard::connect` ([ORB-10708]): pick a local port, open a bare `ssh -N`
forward, probe through it, attach if something already answers, otherwise run a
second `ssh` that both forwards and starts the remote process, and tear down on
drop only what this invocation started. Roughly 150 lines, and every line of it
is exactly what `orbit mcp serve --mode remote` needs.

`orbit-dashboard` already depends on `orbit-remote`. The proxy lives in
`orbit-remote`, so it cannot call into the dashboard: that edge runs the wrong
way, and reversing it would invert the layering for a process-spawning helper.

## Decision

Move the mechanism to `orbit-common::utility::ssh_tunnel` and make both surfaces
consume it. The module owns the `SshTunnel` RAII child, teardown, port
selection, forward-argument construction, `shell_quote`, `ssh` exit
classification, readiness polling, and the attach-first `establish` sequence.

Each consumer keeps only what is genuinely its own: the dashboard keeps its
`/healthz` probe, its remote `orbit web serve` command line, and its browser and
shutdown behavior; the proxy keeps its TCP readiness probe, its
`orbit mcp serve --listen` command line, and its checkout guard. A `TunnelSpec`
carries the caller's remote command and its two timeouts, so the shared module
never composes what runs on the far side.

The module is deliberately synchronous and `std`-only. A tunnel is a process
lifetime, not a future; consumers own their own runtime, or have none.

The leaf is the placement, not `orbit-exec` (whose process primitives are about
sandboxed command execution under an `FsProfile`) and not a new crate. Both
consumers already depend on `orbit-common`, so this adds **no new dependency
edge**.

## Consequences

- The "one mechanism" property in [ADR-0350] is now structural rather than
  aspirational: a third loopback listener reaching for a tunnel finds one
  implementation, and a fix to teardown or attach semantics lands for every
  consumer at once.
- Attach-first behavior — the part that makes a long-lived remote listener
  usable at all — is inherited by the proxy rather than reimplemented, so the
  two surfaces cannot drift on whether disconnecting kills a pre-existing
  remote process.
- Generic behavior is tested once, in `orbit-common`; each consumer's tests
  shrink to the part it actually owns.
- **Cost:** `orbit-common` is a `stable`-tier leaf and now spawns processes.
  That is a genuine widening of what "shared utility" means there, justified
  only because the alternative placements are worse: duplicating ~150 lines
  across two crates exceeds the duplication threshold, and an
  `orbit-remote -> orbit-dashboard` edge inverts the layering.
- **Cost:** the dashboard's timeout and `ssh`-exit messages are now composed
  from a shared template plus a caller-supplied description, so their exact
  wording changed slightly. Operator-facing strings are no longer owned
  end-to-end by the command that emits them.
- **Rejected:** duplicating the helpers into `orbit-remote` and filing a
  follow-up consolidation task. It is the cheaper edit and the standard escape
  hatch for cross-crate duplication, but it contradicts [ADR-0350]'s explicit
  "rather than opening a second mechanism" the moment the second consumer
  exists — the exact point at which consolidating is still cheap.
- **Rejected:** a dedicated `orbit-tunnel` crate. Correct if a third consumer
  with different transport needs appears; today it buys isolation nothing
  currently needs at the cost of a crate in the graph.