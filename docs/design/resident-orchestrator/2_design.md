---
title: Resident Orchestrator — Design
owner: codex
last_updated: 2026-07-17
status: Draft
feature: resident-orchestrator
doc_role: design
type: design
summary: CLI-backed pickup, checkpoint, decomposition, and shepherding contract for workspace-resident orchestrators.
tags: [resident-orchestrator, epic, routines, cli]
paths: [".orbit/resources/activities/**", ".orbit/resources/jobs/**", ".orbit/routines/**", "crates/orbit-core/assets/**"]
related_features: [resident-orchestrator, activity-job, routines, agent-families, host-registry]
related_artifacts: []
---

# Resident Orchestrator — Design

This document specifies the proposed resident-orchestrator contract. It covers how a high-level
assignment is addressed, selected, resumed, decomposed, dispatched, and closed using Orbit's CLI
backend and durable task state. It does not change leaf implementation pipelines or introduce a
general distributed workflow engine.

## 1. Addressing Work Through the Workspace

The destination workspace is the agent address. The front-door orchestrator delegates by creating
one task in that workspace with:

- no `parent_id`;
- tag `epic`;
- an outcome-oriented description;
- observable epic-level acceptance criteria;
- the appropriate priority; and
- `proposed` or `backlog` status under the workspace's normal approval policy.

There is no required `assignee` field. In v1, a workspace has at most one configured resident
orchestrator, so workspace routing plus the `epic` tag is unambiguous. The task remains a normal
Orbit task: it can be searched, blocked, rejected, archived, related to other artifacts, and
inspected by any operator.

The `epic` tag is deliberately behavioral only at the resident pickup boundary. It does not alter
the task schema or create a new task type. Child tasks are recognized by `parent_id`, not by a
special child tag.

## 2. Workspace-Local Resident Identity

Each participating workspace owns an activity named `resident_orchestrator`. The activity is the
declarative invocation profile for that workspace's specialized agent:

```yaml
schemaVersion: 2
kind: Activity
metadata:
  name: resident_orchestrator
spec:
  type: agent_loop
  description: Run one bounded ownership cycle for this workspace's resident orchestrator.
  backend: cli
  provider: codex
  model: gpt-5.6-sol
  max_iterations: 1
  wall_clock_timeout_seconds: 7200
  instruction: |
    Read the resident identity and active rules before acting.
    You own one bounded epic-shepherding cycle in this workspace.
```

The example values are illustrative; each workspace chooses its own provider, model, and identity
instruction. The resolved provider/model must be explicit and must match the named resident's
identity. Defaults or role overrides that silently change the resolved identity are invalid for a
resident activity.

Identity memory remains outside Orbit's task store. The activity instruction points the CLI agent
to its versioned memory layer, and the CLI harness loads the workspace's normal repository
instructions. Orbit owns invocation, task state, and audit evidence; it does not become an agent
memory service.

Using a normal activity asset is intentional. It reuses the existing CLI executor and lets every
workspace version its own resident binding without adding an identity registry or another server.

The first Constellation canary is `ws_orbit`: Hohmann is bound explicitly to Codex Sol and loads
its versioned memory layer before each cycle. Adopting this design changes Hohmann from a leaf that
returns review/merge/closure to the front door into a codebase-bounded orchestrator that owns those
steps inside `ws_orbit`. The front door still owns cross-workspace routing, product priority, and
any independent oversight required by live policy.

## 3. The Resident Epic Cycle

A new `resident_epic_cycle` job performs one bounded ownership cycle:

1. **Select.** A deterministic `select_resident_epic` activity searches the source workspace.
2. **Invoke.** When a task is selected, the job invokes `activity:resident_orchestrator` with the
   workspace identity, parent task snapshot, known child summaries, and relevant run pointers.
3. **Record.** The CLI agent performs task and workflow operations through Orbit's managed tool
   surface; those writes are the checkpoint.
4. **Exit.** The job ends after the resident reports the cycle outcome. It does not sleep while a
   child workflow is running and does not retain a provider session for the next fire.

Selection order is deterministic:

1. resume the oldest `in-progress`, root, `epic` task;
2. otherwise select the highest-priority ready `backlog` root epic, then creation order;
3. otherwise select a `proposed` root epic only when workspace policy permits the resident to plan
   and start it; and
4. otherwise return a successful no-op.

The first version permits one active epic per workspace. The routine uses `overlap: forbid`, pins
one host, and the selector always resumes active ownership before admitting new work. These three
constraints avoid a new lease or assignee subsystem. A concurrent manual lifecycle transition is
resolved by the task store's existing status validation; the losing cycle refreshes instead of
forcing state.

## 4. Resident Cycle Contract

The resident is an orchestrator within one workspace, not a leaf implementer and not a global
supervisor. During each cycle it must:

1. load the parent task, its plan, acceptance criteria, children, dependencies, review threads,
   artifacts, and active workflow runs;
2. author or refine the parent plan before starting an unplanned proposal;
3. start the parent when it accepts ownership;
4. create independently shippable child tasks with `parent_id`, strong acceptance criteria,
   precise `context_files`, dependencies, priority, complexity, and crew;
5. dispatch only explicit ready child IDs through the workspace's normal shipment workflow;
6. observe existing child runs before dispatching anything, so a restart never duplicates an
   in-flight shipment;
7. obtain and enforce the workspace's independent-review policy;
8. resolve failures, review findings, merge conflicts, and stale branches within its workspace;
9. verify landed commits and child lifecycle state rather than trusting agent prose or a PR merge
   button; and
10. complete the parent only after every required child is terminal and the parent-level acceptance
    criteria have been verified as an integrated outcome.

The resident may inspect adjacent workspaces to understand an interface, but it must not silently
edit or dispatch into them. A cross-workspace dependency becomes a separate epic assignment in the
destination workspace, related from the source task with explicit workspace and task pointers.
The front-door orchestrator remains the authority for cross-workspace priority and product scope.

## 5. Durable Checkpoints and Resumption

No correctness may depend on CLI conversation history. The recovery state is the Orbit graph:

- the parent task's status, plan, comments, execution summary, and acceptance criteria;
- child tasks connected by `parent_id`;
- child `dependencies` and relations;
- workflow/run IDs attached to tasks or recorded in artifacts/comments;
- review threads and structured verdicts; and
- repository/PR state verified during the cycle.

The resident writes a concise cycle checkpoint to the parent before exiting. At minimum it records
what changed, which children or runs remain active, the next safe action, and any blocker requiring
new authority. Checkpoints are pointers, not copied logs.

Crash behavior follows from where the failure occurs:

| Failure point | Durable result | Next fire |
|---------------|----------------|-----------|
| Before selection | No task mutation | Select normally |
| After selection, before start | Parent remains proposed/backlog | Re-evaluate and retry |
| After parent start | Parent remains `in-progress` | Resume it before new work |
| After child creation | Children remain attached | Continue decomposition/dispatch |
| After workflow submit | Run/task link remains authoritative | Observe; do not redispatch |
| While waiting for review or CI evidence | Parent remains active | Recheck only the required gate |
| Genuine product-authority blocker | Parent moves to `blocked` with exact question | Stop automatic retries |

## 6. Routine Contract

Each resident workspace may enable a versioned routine targeting `job:resident_epic_cycle`:

```yaml
schemaVersion: 1
name: resident-epic-orbit
enabled: true
hosts: [dk-server-1]
trigger: { cron: "*/5 * * * *", missed_run: skip }
target: job:resident_epic_cycle
policy:
  timeout_minutes: 120
  overlap: forbid
```

The routine is only a clock and admission boundary. It must not discover or ship ordinary backlog
tasks. An `epic` root was deliberately placed in this workspace by an upstream orchestrator or
human, so pickup is explicit delegation rather than blind auto-dispatch.

Routine definitions remain disabled by default when seeded. Enabling a resident is a versioned
workspace decision, and the activity must resolve to a valid CLI provider/model on the pinned host
before enablement.

## 7. Child Shipment and Completion

The resident reuses existing leaf delivery workflows. It does not implement code inside the epic
cycle unless the workspace's delivery policy explicitly treats a bounded change as direct work.
Normally it promotes ready children and invokes shipment with explicit task IDs, selected mode,
crew, base branch, and independent review configuration.

An epic may have sequential and parallel children. Ordering is expressed durably through child
`dependencies`; disjoint ready children may be shipped concurrently within the workspace's normal
run and lock limits. Dependency inference that exists only in a model's reasoning is incomplete —
the resident must write it to the child tasks before dispatch.

The parent does not become `done` merely because all children reached `review`. Completion requires:

- every required child is `done`, `rejected`, or explicitly accepted as out of scope;
- no open blocking review thread remains;
- required PRs are merged and candidate commits are on the integration branch;
- parent-level integration checks pass; and
- the parent execution summary contains a structured final verdict and evidence pointers.

## 8. Phasing Out the HTTP Epic Pipeline

`task_epic_pipeline` and its `epic_orchestrator` activity are legacy after the resident CLI path is
available. They should not be converted in place because their contract differs materially:

| HTTP epic path | Resident CLI path |
|----------------|-------------------|
| Requires pre-existing children | Owns decomposition and child creation |
| Uses `backend: http` and a retained `session:` loop | Uses bounded `backend: cli` cycles |
| Keeps progress partly in provider conversation state | Derives progress from durable Orbit state |
| Dispatches child gates | Shepherds dispatch, review, merge, and closure |
| Treats `review` as shipped/terminal | Requires verified task and integration completion |

Retirement proceeds in four stages:

1. ship the resident selector, CLI activity contract, cycle job, and disabled routine;
2. canary one workspace and verify crash/resume, duplicate-dispatch prevention, review repair, and
   final parent closure;
3. remove `task_epic_pipeline` from seeded/default catalog references and migrate active users; and
4. delete the HTTP-only epic activities/actions once no live run or workspace references them,
   updating the Activity / Job decisions and task references in the same change.

Historical run records remain readable after catalog retirement. Removal must fail closed for a
workspace that still has an enabled routine or job reference to the legacy pipeline.

## 9. Security and Authority

`backend: cli` delegates tool enforcement to the provider harness, so the resident activity is a
trusted workspace capability. The routine therefore requires the same review boundary as any
versioned automation asset, and the CLI run must retain Orbit's managed run identity and audit
envelope.

The resident's authority is bounded by the source workspace and live repository instructions.
It may exercise routine lifecycle operations needed to shepherd already-authorized work, but it
must block and surface a precise question when completion needs new product authority or a material
scope expansion.

## 10. Concerns & Honest Limitations

- **Workspace ownership is singular in v1.** Multiple resident orchestrators in one workspace need
  an explicit routing or lease design; tags alone are not enough.
- **Polling adds latency.** A five-minute cadence is simple and robust but delays pickup and
  resumption. Event triggers remain outside routines v1.
- **CLI cycles rehydrate context.** Durable state prevents correctness loss, but each invocation
  pays the cost of reading identity, task state, and repository context again.
- **Cross-workspace epics are not one graph.** Each workspace owns its own task tree; the front door
  must relate and observe multiple parent tasks when one product outcome spans codebases.
- **Provider harness trust is real.** CLI activities do not receive the HTTP backend's builtin tool
  allowlist enforcement. Host sandboxing and provider policy remain part of the security boundary.
- **A resident can still make poor decomposition choices.** Deterministic selection and durable
  state make those choices inspectable and recoverable; they do not eliminate judgment risk.
- **Legacy removal needs migration evidence.** Deleting the HTTP epic assets before all routine,
  job, and live-run references are checked would turn a design cleanup into an operational break.

## Task References

- None yet — implementation tasks will be allocated after this Draft is accepted.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
