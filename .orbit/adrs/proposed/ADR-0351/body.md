## Context

With the tunnel owned as infrastructure, an off-box orchestrator can reach the remote machine. The question is what it should be able to do there.

A correction first, because an earlier draft of this record overstated the problem. Orbit's MCP tool definitions are **not** hand-maintained duplicates of the CLI. They are derived from the tool registry — a tool is written once in Rust with its schema, registered with a policy, and the advertised surface is computed from those entries. The duplication this feature was created to remove was an external process re-declaring those schemas in another language across a process boundary, and the owned tunnel already eliminates it. What the advertised surface actually costs is per-tool policy and placement metadata, the conformance test pinning the definition count, the contract digest, and the context those definitions occupy in every client request. Real, but modest.

What is genuinely scarce is reachability. An orchestrator that cannot execute on the machine routes trivial reads through full worker runs — disproportionate to the work, and slow enough to distort how often such checks happen at all.

There is also a boundary to respect. A client that can run arbitrary commands can invoke the CLI, and the CLI reaches every operation the capability model governs, including workflow dispatch. Unrestricted command execution in the default surface would make capability filtering, the governed-operation check, and the workspace claim advisory for whoever holds it. Against that: establishing the tunnel already presupposes SSH to the machine, and anyone with SSH can already run anything there.

## Decision

Add command execution, and change nothing else about the surface.

- **Command** takes an argv array and an explicit working directory. Never a shell string, so quoting and operator-precedence bugs are structurally impossible rather than merely discouraged.
- It requires **operator capability and the workspace claim**, and is withheld from managed runs, which could otherwise bypass the self-dispatch guard through the CLI.
- A client without the claim does not receive command at all. The restriction is **not** an allowlist over argv: a filtered command surface leaks through `bash -c`, `env`, `xargs`, `make`, interpreter `-c` flags, and version-control hooks, so the boundary is whether the operation exists for that caller, not which binaries it may name.
- **The advertised per-tool surface is unchanged.** Clients keep native tool selection, call-time argument validation, and per-tool audit attribution. Routine work continues to be attributed by tool name; only genuinely arbitrary execution degrades to an argv.

Replacing the advertised surface with generic enumerate and invoke-by-name operations is deliberately **not** decided here. It remains open, and the cost of keeping it open is one additional path to the same operations.

## Consequences

- The orchestrator stops dispatching a full worker run to answer questions a single command answers, and new CLI capability is reachable the moment it ships rather than after a schema is mirrored.
- Per-tool audit attribution is preserved for everything except command itself, which is the narrowest possible degradation of provenance.
- Nothing is foreclosed. The advertised definitions are generated from the registry, so removing them later is a revert rather than a rebuild — that, not a measurement, is what makes this reversible.
- **Cost:** for a claim-holding client, capability filtering above command is advisory — it can invoke the CLI and reach any governed operation. Requiring both operator capability and the claim, and withholding it from managed runs, bounds who that applies to; it does not make it untrue.
- **Cost:** audit granularity degrades for command calls specifically. An argv is not a tool name, and workspace correlation becomes conventional rather than structural.
- **Cost:** two paths now reach the same operations — the advertised tool and the CLI through command. That duplication is accepted deliberately rather than by oversight.
- **Cost:** deciding later whether the advertised surface earns its place requires evidence that no current endpoint produces. `/metrics/tools` is an ungrouped invocation count with no caller dimension; the usable cut is over audit events, excluding rows carrying a job-run or activity id so that engine and worker traffic does not swamp the orchestrator's. Until someone builds that cut, retaining the surface is deferral, not measurement, and this record should not pretend otherwise.