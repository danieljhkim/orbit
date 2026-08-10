## Context

With the tunnel owned as infrastructure, an off-box orchestrator can reach the remote machine. The question is what surface it should find there.

A correction first, because an earlier draft of this record overstated the problem. Orbit's MCP tool definitions are **not** hand-maintained duplicates of the CLI. They are derived from the tool registry — a tool is written once in Rust with its schema, registered with a policy, and the advertised surface is computed from those entries. The duplication this feature was created to remove was an external process re-declaring those schemas in another language across a process boundary, and the owned tunnel already eliminates it. What Orbit's own advertised surface actually costs is per-tool policy and placement metadata, the conformance test that pins the definition count, and the contract digest. That is a real but modest cost, and decisions here should be sized against it rather than against a duplication that does not exist.

What remains genuinely scarce is reachability. An orchestrator that cannot execute on the machine routes trivial reads through full worker runs — disproportionate to the work, and slow enough to distort how often such checks happen at all.

There is also a boundary to respect. A client that can run arbitrary commands can invoke the CLI, and the CLI reaches every operation the capability model governs, including workflow dispatch. Unrestricted command execution in the default surface would make capability filtering, the governed-operation check, and the workspace claim advisory for whoever holds it. Against that: establishing the tunnel already presupposes SSH to the machine, and anyone with SSH can already run anything there.

## Decision

Serve three operations over the tunnel, and let them carry different authority.

- **Enumerate** — return the registry entries visible to this caller, with descriptions and input schemas, filtered by capability and by workspace claim. This preserves discoverability without advertising one MCP definition per tool.
- **Invoke by name** — take a tool name, an input object, and a workspace, and dispatch through the existing governed chokepoint. Authorization by capability, placement, and claim happens at invocation rather than by omission from an advertised list. The audit record carries the inner tool name, workspace, and capability, so provenance is unchanged from the advertised surface.
- **Command execution** — take an argv array and an explicit working directory. Never a shell string, so quoting and operator-precedence bugs are structurally impossible rather than discouraged. This operation requires **operator capability and the workspace claim**, and is withheld from managed runs, which could otherwise bypass the self-dispatch guard through the CLI.

A client without the claim receives enumerate and invoke, never command. The restriction is **not** an allowlist over argv: a filtered command surface leaks through `bash -c`, `env`, `xargs`, `make`, interpreter `-c` flags, and version-control hooks, so the boundary is which operations exist for that caller, not which binaries it may name.

The existing advertised per-tool surface is **retained for now**. It is generated rather than hand-written, so keeping it is cheap; tool-call metrics decide whether it earns its place, and removing it later is a small change. Removing it now is the irreversible half of the decision and buys the least.

## Consequences

- The orchestrator stops dispatching a worker run to answer questions one command answers, and every registry operation stays reachable without a per-operation adapter.
- Splitting invoke from command keeps audit granularity that a command-only surface would have discarded: routine work is attributed per tool, and only genuinely arbitrary execution degrades to an argv.
- Policy and placement metadata move from filtering an advertised list to authorizing an invocation — enforcement rather than advertisement, which is where they bind.
- **Cost:** enumerate and invoke reproduce the protocol's own listing and call verbs inside a tool. That is a protocol within a protocol, and it is only justified if collapsing per-tool policy surface into one authorization point is worth the indirection. If the answer to "why not advertise the tools" is only "we would rather not maintain per-tool policies," that is worth confronting rather than routing around.
- **Cost:** for a claim-holding client, capability filtering above command is advisory — it can invoke the CLI directly. Requiring both operator capability and the claim bounds who that applies to; it does not make it untrue.
- **Cost:** audit granularity still degrades for command calls specifically. An argv is not a tool name, and workspace correlation becomes conventional rather than structural.
- **Cost:** retaining the advertised surface means two ways to reach the same operation until the measurement resolves it, and the deferral only pays off if the metrics are read.