## Context

With the tunnel owned as infrastructure, an off-box orchestrator can reach the remote machine. The question is what it should be able to do there.

Orbit's CLI is already a complete surface: every registered tool is reachable through it, and it is the surface a human uses. The MCP tool set is a hand-maintained adapter over that same functionality — schemas, capability policies, placement metadata, and an advertised list, all kept in step by hand. That is the duplication this feature was created to remove, one layer up from Bridge. Agents drive command-line interfaces competently; the assumption that they need a typed tool per operation is not supported by how they are actually used.

Reachability is also the scarce resource in practice. An orchestrator that cannot execute on the machine currently routes trivial reads through full worker runs, which is disproportionate to the work and slow enough to distort how often such checks happen at all.

Against that: a client holding shell can invoke the CLI, and the CLI can perform every operation the capability model governs. Shell in the default surface would make capability filtering, the governed-operation check, and the managed-run self-dispatch guard advisory for anyone holding it.

The countervailing fact is where trust already sits. Establishing the tunnel requires SSH to the machine, and anyone with SSH can already run anything there. Orbit declining to expose command execution to such a caller protects nothing; it only makes the caller less efficient. The boundary that matters is the one SSH already enforces.

## Decision

Add a command-execution tool that runs on the remote machine through the owned tunnel, gate it, and use it to settle the surface question empirically.

- Input is an **argv array with an explicit working directory**, not a shell string. No shell interpolation, so quoting and operator-precedence bugs are structurally impossible rather than merely discouraged.
- The tool requires **operator capability** and is registered **recognized-but-not-advertised**, the treatment already used for administrative tools. It does not appear in the advertised list, and a guessed call reaches the audited denial path rather than an unknown-tool error.
- It is **not part of the default agent surface**, and is not granted to managed runs. A managed run holding it could bypass the self-dispatch guard by invoking the CLI.
- The audit record carries argv, working directory, caller, and workspace.
- **The existing tool surface is not removed by this decision.** Skills point at the CLI, tool-call metrics are collected, and the set of first-class tools is revisited from observed use.

## Consequences

- The orchestrator can inspect and operate the remote machine directly, removing the pattern of dispatching a full worker run to answer a question a single command answers.
- Every operation stays reachable without a per-operation adapter, so new CLI functionality is available immediately rather than after a schema is mirrored.
- The surface question becomes answerable from evidence. Existing tool metrics record which tools are actually called, so the decision to retire the adapter can rest on observed use rather than on prediction.
- **Cost:** for a client holding this tool, capability filtering above it is advisory — it can invoke the CLI and reach operations the model would otherwise gate. This is accepted because the tunnel already presupposes SSH, and is precisely why the tool is operator-only, unadvertised, and withheld from managed runs.
- **Cost:** audit granularity degrades for these calls. An argv is not a tool name, and correlation to a workspace or task is by convention rather than by structure. Runtime-level audit still records what the CLI itself performs, so the loss is at the MCP layer rather than total.
- **Cost:** the surface stays duplicated through the measurement period. That is the price of not freezing a contract prematurely, and it is only worth paying if the measurement is actually read and acted on.
- If measurement shows the command tool is the only surface in real use, the honest follow-on question is not which tools to delete but whether Orbit should ship an MCP server at all rather than a tunnel and a skill. This decision does not answer that, but it makes it reachable deliberately instead of by drift.