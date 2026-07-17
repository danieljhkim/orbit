# Glossary — Host Registry

Vocabulary specific to the host-registry feature. Excludes standard industry terms
(heartbeat, label, inventory, lease TTL) except where Orbit assigns them a narrower
meaning. Standard Orbit terms (workspace, crew, sweep) are defined in their own
features.

| Term | Meaning |
|------|---------|
| Caller-host provenance | The caller's `machine_id`/`host_id` carried in MCP session metadata and stamped onto audit rows; distinct from the existing audit `host` column, which records the hostname of the executing process. See [2_design.md §2](../2_design.md). |
| Coordination plane | The record types that live only on the main host for every workspace: tasks, review threads, artifacts, frictions, the run queue, the registry, and all global ID allocation. A v1 invariant, not per-workspace configuration. See [2_design.md §3](../2_design.md). |
| Execution host | The machine a specific task's agent run is placed on; per-task, orchestrator-selected, validated against the registry like `crew`. See [2_design.md §4](../2_design.md). |
| Host identity | The machine-local declaration in `~/.orbit/host.toml`: generated, immutable `machine_id` plus renameable human `host_id`; initialized by `orbit init`. See [2_design.md §1](../2_design.md). |
| Host registry | The main host's inventory of registered machines (name, machine id, labels, workspace presence map, status, last-seen), enumerable via `orbit.host.list`. See [2_design.md §2](../2_design.md). |
| Local routine | A routine definition under `.orbit/routines/local/` (gitignored by convention); implicitly pinned to the machine it sits on. See [2_design.md §6](../2_design.md). |
| Main host (hub) | The machine holding the coordination plane; in the current constellation `dk1`, which also owns nearly every workspace. A mailbox, not a relay: it queues placed runs, satellites collect their own. See [1_overview.md §2](../1_overview.md). |
| Manual execution | The non-shipped path: a human claims a task (excluding it from ship triage), works in a local checkout, lands code through the repo's gate, and resolves the task citing the PR/commit. No run, lease, or placement snapshot exists. See [2_design.md §4](../2_design.md). |
| Placement snapshot | The immutable `placement.requested` (name as written + resolved `machine_id`) and `placement.actual` (leasing host's `machine_id`) fields on each run; retries re-resolve the task-level preference. See [2_design.md §4](../2_design.md). |
| Registry cache | A satellite's local snapshot of registry data, refreshed on each successful poll/register; read by validation only (warning-only when stale), never by enforcement. See [2_design.md §3](../2_design.md). |
| Replica mode | The posture of a non-owner checkout: local CLI refuses knowledge-record authoring for that workspace, decided entirely from the machine's own workspace entry. (Coordination mutations are separately refused machine-wide on every machine but the hub.) See [2_design.md §3](../2_design.md). |
| Routine ownership rule | The host pinned in a git-committed routine definition is the host in charge of that routine; unpinned committed definitions fail closed. See [2_design.md §6](../2_design.md). |
| Run lease | The pull-based hub→satellite protocol: satellite-placed runs wait in `placed`; the satellite's minute-cadence poller calls `orbit.run.lease`, executes locally, reports via `orbit.run.report`. Expired leases return to `placed`. See [2_design.md §4](../2_design.md). |
| Runner capability set | The narrow MCP capability set for satellite pollers: lease, report, presence refresh — no shipping, host enumeration, or record authoring. See [2_design.md §4](../2_design.md). |
| Satellite | Any registered host other than the main host. Satellites initiate every connection they participate in; nothing connects to them. See [2_design.md §3](../2_design.md). |
| Tombstone alias | A renamed or retired `host_id` kept in the registry, resolving-with-warning to its original `machine_id`; never claimable by a different machine. See [2_design.md §2](../2_design.md). |
| Workspace owner | Per workspace, the single machine holding the canonical checkout: default execution host and sole author of that workspace's knowledge records. A declared binding recorded on the hub's workspace entry and mirrored locally at link time. See [2_design.md §3](../2_design.md). |
| Workspace presence map | Per registry entry: `{workspace_id → {root, last_verified}}` on that host, reported at registration and refreshed on each poll; placement validation uses the map, while the leasing satellite resolves the stable workspace ID through its own local registry. See [2_design.md §2](../2_design.md). |
