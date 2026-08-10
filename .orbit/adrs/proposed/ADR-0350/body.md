## Context

Orbit's canonical MCP surface reaches a remote machine one way today: a spoke broker spawns `ssh <alias> orbit mcp serve --hub` and relays frames over that process's stdio. The stated posture is that Orbit opens no listening port and invents no credential of its own.

That path assumes the client is a spoke — a machine with its own checkout, whose graph, docs, and search must resolve against the branch its agent is working on. Placement classes (`hub`, `owner`, `local-derived`, `composite`) exist to preserve exactly that.

A second client class does not fit the assumption. An off-box orchestrator has no meaningful local checkout: its clone, if any, is a read mirror, and every workspace it acts on lives on the remote. There is no local-derived state to protect, so placement routing guards nothing for it and only makes the canonical surface unreachable. The observed consequence is the parity layer this feature already decided to retire — an external process that re-declares Orbit's tools against the dashboard HTTP API, discards Orbit's capability model, and drifts on every schema change.

Two further facts shape the decision. First, reachability is the scarce thing, not tool schemas: an orchestrator that cannot reach the machine currently launders trivial reads through full worker runs. Second, Orbit's CLI is already a complete surface, and the MCP tool set is a hand-maintained adapter over it — the same duplication this feature exists to remove, one layer up. Freezing a tool contract now would repeat that mistake with more confidence than the evidence supports.

## Decision

Treat the SSH tunnel as owned infrastructure, and treat the surface carried over it as provisional.

- Orbit establishes or reuses an SSH tunnel to a **loopback-bound** listener on the remote machine. The listener refuses any non-loopback bind, exactly as the dashboard does. SSH owns authentication, encryption, and host verification; Orbit adds no credential, ACL, or session of its own. This is the same delegation the hub link already makes, applied to a tunnel rather than a spawned process.
- The tunnel is a reusable primitive, not an implementation detail of one consumer. Anything that needs to reach the remote machine rides it rather than opening a second mechanism.
- For a **checkoutless client**, forwarding is verbatim: frames are relayed unchanged and every placement class resolves remotely. The mode must refuse to start where a local checkout exists, rather than silently answering from another machine's branch.
- Placement routing is unchanged for spokes. This narrows the placement broker's scope to clients that hold local-derived state; it does not supersede that decision.
- **The composition of the surface carried over the tunnel is deliberately not settled here.** Which operations remain first-class and which are better reached through the CLI is a question to answer from tool-call metrics after the tunnel is in use, not by prediction now.

## Consequences

- The canonical surface becomes reachable off-box with no re-declaration. Schema drift stops being possible for forwarded operations because both ends are the same build.
- Capability filtering and audit apply to remote callers through the paths that already implement them, rather than needing equivalents rebuilt on a second surface.
- **Cost:** Orbit now opens a listening port, contradicting a previously absolute posture. Loopback binding plus a tunnel preserves the security property, but that guarantee now rests on a bind guard rather than on the absence of a listener — and a misconfiguration binding a routable address turns the surface into unauthenticated remote control of the machine.
- **Cost:** a second cross-machine mechanism exists beside the SSH-stdio hub link. Until one is retired, two paths reach a remote Orbit, which is the duplication this feature was created to remove. The tunnelled listener is deliberately not a hub link and must not acquire hub-link responsibilities: no placement routing, no workspace-ownership resolution, no spoke registration.
- **Cost:** verbatim forwarding is correct only for the client class it is defined for. The refusal-when-a-checkout-exists guard is load-bearing; without it the mode returns another machine's branch state as though it were local, which presents as wrong answers rather than as an error.
- **Cost:** declaring the surface provisional means accepting churn. Clients may see the advertised set change, and the duplicated surface persists through the measurement period rather than being resolved by this decision.
- The star topology's "one cross-machine destination" invariant now describes spokes specifically. A checkoutless client is not a spoke and does not participate in hub or owner routing.