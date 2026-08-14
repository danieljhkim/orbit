---
title: Resident Orchestrator — Design
owner: grok
last_updated: 2026-08-14
status: Accepted
feature: resident-orchestrator
doc_role: design
type: design
summary: Bounded, resumable CLI cycles for workspace-resident orchestration, durable decision gates, decomposition, and shepherding.
tags: [resident-orchestrator, epic, routines, cli, decision-gates]
paths: [".orbit/resources/activities/**", ".orbit/resources/jobs/**", ".orbit/routines/**", "crates/orbit-core/assets/**"]
related_features: [resident-orchestrator, activity-job, routines, agent-families, host-registry]
related_artifacts: [ORB-10332, ORB-10775, ORB-10776, ADR-0352, ADR-0361]
---

# Resident Orchestrator — Design

This document specifies the accepted resident-orchestrator contract. It covers how a high-level
assignment is addressed, selected, resumed, clarified, decomposed, dispatched, and closed using
Orbit's CLI backend, a resumable provider conversation, and durable task state. It does not change
leaf implementation pipelines or introduce a general distributed workflow engine.

The v1 hierarchy is human → front-door supervisor → workspace resident orchestrator → leaf
executor. The resident minimizes the supervisor's workspace-local surface area, but it is not the
product owner: product scope, material tradeoffs, and merge authority stay at the supervisor or
human layer. V1 uses bounded headless CLI processes. It deliberately excludes managed PTYs,
terminal attachment, and live mid-turn steering.

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

This differs from the former `task_epic_pipeline`, whose `load_epic` path recognized a root by
`TaskType::Feature`. That HTTP epic pipeline was removed as unused in [ORB-10332], so the resident's
`epic` tag is now the only epic selector; the earlier plan to keep the two selectors disjoint by
workspace during a staged retirement no longer applies. The marker choice is named in
[4_decisions.md](./4_decisions.md).

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
  wall_clock_timeout_seconds: 7200
  instruction: |
    Read the resident identity and active rules before acting.
    You own one bounded epic-shepherding cycle in this workspace.
```

The example values are illustrative; each workspace chooses its own provider, model, and identity
instruction. The resolved provider/model must be explicit and must match the named resident's
identity. Defaults or role overrides that silently change the resolved identity are invalid for a
resident activity.

`AgentLoopSpec.max_iterations` in `activity_v2.rs` bounds turns in Orbit's HTTP loop. The CLI
dispatcher launches one provider process and does not apply that field to provider turns, so the
resident example omits it: a full shepherd cycle is bounded by the 7,200-second wall clock, while
provider turn knobs remain provider/harness policy rather than Orbit activity configuration.

Identity memory remains outside Orbit's task store. The activity instruction points the CLI agent
to its versioned memory layer, and the CLI harness loads the workspace's normal repository
instructions. Orbit owns invocation, task state, and audit evidence; it does not become an agent
memory service.

Using a normal activity asset is intentional. It reuses the existing CLI executor and lets every
workspace version its own resident binding without adding an identity registry or another server.

The resident activity also supports an opaque provider conversation reference. The first cycle
starts a new conversation and records its reference; later cycles resume it when the workspace,
provider, model, and resident identity still match. Conversation continuity reduces repeated
orientation cost and preserves the semantic thread across decisions, but it grants no authority
and is never the source of truth for task or workflow state.

The first Constellation canary is `ws_orbit`, bound explicitly to crew `grok` (Grok Build /
the workspace Grok default model) [ORB-10782]. The resident loads its versioned memory layer
before each cycle and owns decomposition, shipment, review-wait, and closure inside `ws_orbit`.
The front door still owns cross-workspace routing, product priority, and any independent
oversight required by live policy. The earlier Hohmann / Codex Sol binding was an illustrative
example and is not the canary.

## 3. The Resident Epic Cycle

A new `resident_epic_cycle` job performs one bounded ownership cycle:

1. **Select.** A deterministic `select_resident_epic` activity searches the source workspace.
2. **Rehydrate.** The job loads the latest durable epic state and any compatible provider
   conversation reference. An unresolved decision request without an answer returns a successful
   no-op before provider invocation.
3. **Invoke.** The job starts one headless CLI process. It either creates a provider conversation or
   resumes the compatible conversation with the fresh parent snapshot, child summaries, relevant
   run pointers, and any newly recorded decision answer.
4. **Record.** The CLI agent performs task and workflow operations through Orbit's managed tool
   surface. The provider adapter captures the conversation reference from the provider stream, and
   the job writes a concise checkpoint containing that reference and the cycle outcome.
5. **Exit.** The job ends. It does not sleep while a child workflow or human decision is pending,
   and no provider process remains running between fires.

The retained object is a resumable conversation reference, not a resident process. If that
reference is missing, expired, incompatible, or cannot be resumed, the job starts a new provider
conversation from durable Orbit state and records the replacement. Loss of provider conversation
history may increase reorientation cost, but it must not change the safe next action.

Selection order is deterministic:

1. resume the oldest `in-progress`, root, `epic` task;
2. otherwise select the highest-priority ready `backlog` root epic, then creation order;
3. otherwise return a successful no-op.

V1 does not pick up `proposed` epics. A human or upstream authority must approve one into `backlog`
before the resident can claim it; a future policy surface for proposed pickup remains an open
question in [3_vision.md](./3_vision.md).

The first version permits one active epic per workspace. The routine uses `overlap: forbid`, pins
one host, and the selector always resumes active ownership before admitting new work. These three
constraints avoid a new lease or assignee subsystem. A concurrent manual lifecycle transition is
resolved by the task store's existing status validation; the losing cycle refreshes instead of
forcing state.

> **Revised by [ADR-0352].** The three constraints above bound concurrent *automated* routine
> fires; they do not arbitrate between interactive operator sessions, which can now reach one
> workspace from several places at once. Workflow dispatch is additionally gated on an exclusive
> workspace claim — see
> [host-registry/2_design.md §3.2](../host-registry/2_design.md). The reasoning here stands for
> what it covers; it is no longer the whole story.

`overlap: forbid` prevents concurrent fires of this routine, not a manual invocation of the same
activity. Status validation is the admission backstop: only one contender can transition the
selected backlog epic to `in-progress`; a loser refreshes, while contenders resuming an already
active epic are made dispatch-safe by the authoritative task/run lookup in Section 4.

## 4. Resident Cycle Contract

The resident is an orchestrator within one workspace, not a leaf implementer, global supervisor,
or product owner. During each cycle it must:

1. load the parent task, its plan, acceptance criteria, children, dependencies, review threads,
   artifacts, and active workflow runs;
2. before decomposition, make the objective, constraints, non-goals, acceptance evidence, and
   material unresolved assumptions explicit in the parent plan;
3. ask for a decision rather than committing substantial work when competing interpretations
   would produce materially different outcomes or require new authority;
4. start the parent when it accepts ownership;
5. create independently shippable child tasks with `parent_id`, strong acceptance criteria,
   precise `context_files`, dependencies, priority, complexity, and crew;
6. dispatch only explicit ready child IDs through the workspace's normal shipment workflow;
7. query the authoritative run-list projection by each ready child task ID before dispatching;
   observe any non-terminal matching run instead of submitting another shipment;
8. treat waiting for human review/merge approval as an explicit gate: record the pending PR and
   required approver evidence, then exit until that evidence exists;
9. resolve failures, review findings, merge conflicts, and stale branches within its workspace;
10. verify landed commits and child lifecycle state rather than trusting agent prose or a PR merge
   button; and
11. complete the parent only after every required child is terminal and the parent-level acceptance
    criteria have been verified as an integrated outcome.

The resident may decide reversible, workspace-local execution details already implied by the epic.
It must surface decisions that change product scope, acceptance criteria, architecture boundaries,
external behavior, material cost, security posture, or delivery authority. The front-door
supervisor may answer within authority explicitly delegated to it; otherwise it routes the request
to the human. The resident never treats silence as expanded authority.

Shipment submission takes explicit child task IDs. Before a Run becomes dispatchable, the shipment
path must persist those IDs on the Run in the same transaction as Run creation; the run-list
projection indexed by task ID is therefore the authoritative run↔task association. Comments,
execution summaries, and resident checkpoints may point to that association, but are never its
source of truth. This makes the pre-dispatch query reliable even if the resident crashes immediately
after submission and before writing its own checkpoint.

For the `ws_orbit` canary, the review gate is Daniel's human merge approval. Planner, implementer,
and reviewer crew labels are activity labels on one resolved provider/model/backend assignment. The resident may prepare
and repair the candidate, but it must enter the human-approval wait state rather than approving or
merging on Daniel's behalf.

The resident may inspect adjacent workspaces to understand an interface, but it must not silently
edit or dispatch into them. A cross-workspace dependency becomes a separate epic assignment in the
destination workspace, related from the source task with explicit workspace and task pointers.
The front-door orchestrator remains the authority for cross-workspace priority and product scope.

## 5. Durable Checkpoints, Decisions, and Resumption

No correctness may depend on CLI conversation history. The recovery state is the Orbit graph:

- the parent task's status, plan, comments, execution summary, and acceptance criteria;
- child tasks connected by `parent_id`;
- child `dependencies` and relations;
- workflow Runs whose transactionally stored task IDs are exposed by the run-list projection;
- review threads and structured verdicts; and
- repository/PR state verified during the cycle.

The resident writes a concise cycle checkpoint to the parent before exiting. At minimum it records
what changed, which children or runs remain active, the next safe action, and any blocker requiring
new authority. It also records the opaque provider conversation reference and the provider, model,
workspace, and resident identity to which that reference is bound. Checkpoints are pointers, not
copied logs.

### 5.1 Progressive clarification

Upfront planning and mid-execution questions are complementary. Before dispatching the first child,
the resident resolves ambiguity that can be discovered cheaply from the epic, repository, existing
ADRs, and supervisor context. Later it asks when new evidence creates a consequential fork.

The escalation heuristic is:

```text
ask when P(wrong branch | current evidence) × remaining rework cost
         > question cost + expected delay cost
```

This is a judgment aid, not a mechanically calculated policy. Low-cost reversible choices remain
with the resident. The point is to prevent long epics from accumulating expensive work behind an
unstated assumption while avoiding human involvement in routine execution details.

### 5.2 V1 decision exchange

V1 represents checkpoints and the exchange as structured parent-task comments rather than
introducing a new message store. Each machine-readable comment body is one JSON object with
`schema_version: 1` and one of three `kind` values: `resident_cycle_checkpoint`,
`resident_decision_request`, or `resident_decision_answer`. Ordinary prose comments remain valid
and are ignored by the resident-state parser. At most one unresolved decision request may exist
for an epic. A request records:

- a stable request ID and epic ID;
- the exact question and why it blocks safe progress now;
- the viable options and material tradeoffs;
- the resident's recommendation, when it has one;
- the smallest safe work that may continue without the answer;
- the current checkpoint and provider conversation reference; and
- the authority required to answer.

The resident writes the request, records a `waiting_decision` cycle outcome, and exits. The parent
remains `in-progress`; `blocked` is reserved for an external dependency or authority gap that
cannot be resolved through this bounded exchange. While the request is unanswered, the selector
returns a successful no-op without paying for another provider invocation.

The supervisor or human answers through Orbit with a `resident_decision_answer` comment that names
the request ID, the chosen direction, the answering authority, and any changed constraints or
acceptance criteria. Only a matching answer from sufficient authority resolves the request;
duplicate or mismatched answers remain visible but do not create a second decision. The next
routine fire supplies the resolved answer and fresh Orbit state to the resumed provider
conversation. V1 does not inject messages into a running turn and does not keep a process alive
while waiting.

For the Grok canary, a new cycle resumes the saved session through the Grok Build CLI's
non-interactive resume surface [ORB-10780]. Other providers (for example Codex's
[CLI resume](https://developers.openai.com/codex/cli/reference)) keep their own argv; that
shape is a provider adapter detail rather than an Orbit task invariant. A resume failure falls
back to a new conversation after recording the failure; the unanswered or answered decision
remains durable in Orbit either way.

### 5.3 Crash and restart behavior

Crash behavior follows from where the failure occurs:

| Failure point | Durable result | Next fire |
|---------------|----------------|-----------|
| Before selection | No task mutation | Select normally |
| After selection, before start | Parent remains proposed/backlog | Re-evaluate and retry |
| After parent start | Parent remains `in-progress` | Resume it before new work |
| After child creation | Children remain attached | Continue decomposition/dispatch |
| After workflow submit, before resident checkpoint | Run already contains the child task ID and appears in the task-indexed run-list projection | Observe the matching run; do not redispatch |
| After decision request, before answer | Parent has one unresolved request | Return a no-op without provider invocation |
| After decision answer, before resume | Parent has a matched answer | Resume the compatible conversation with the answer and fresh state |
| Provider conversation cannot be resumed | Orbit graph and decision records remain intact | Start a new conversation and continue from the durable checkpoint |
| While waiting for agent review or CI evidence | Parent remains active with the pending gate recorded | Recheck only the required gate |
| While waiting for human review/merge approval | Parent remains active with the PR and required approval evidence recorded | Recheck human approval/merge evidence; do not redispatch or complete |
| Unresolvable product-authority blocker | Parent moves to `blocked` with exact missing authority | Stop automatic retries |

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
  timeout_minutes: 135
  overlap: forbid
```

The routine is only a clock and admission boundary. It must not discover or ship ordinary backlog
tasks. An `epic` root was deliberately placed in this workspace by an upstream orchestrator or
human, so pickup is explicit delegation rather than blind auto-dispatch.

The routine is also the v1 wake mechanism for answered questions. No event bus or live channel is
required: an unanswered decision is filtered before provider invocation, while a matching answer
makes the next cycle runnable. The polling interval therefore bounds response latency without
creating idle model cost.

Routine definitions remain disabled by default when seeded. Enabling a resident is a versioned
workspace decision, and the activity must resolve to a valid CLI provider/model on the pinned host
before enablement.

The routine timeout must be strictly greater than the resident activity wall clock plus job and
checkpoint overhead. For the 120-minute activity example, the routine reserves 15 minutes of
headroom (`135` minutes total), so routine-level expiry cannot preempt the activity supervisor's
shutdown and final durable checkpoint. Workspaces that change the activity wall clock must increase
the routine timeout by at least the same delta while retaining explicit headroom.

## 7. Child Shipment and Completion

The resident reuses existing leaf delivery workflows. It does not implement code inside the epic
cycle unless the workspace's delivery policy explicitly treats a bounded change as direct work.
Normally it promotes ready children and invokes shipment with explicit task IDs, selected mode,
crew and base branch.

An epic may have sequential and parallel children. Ordering is expressed durably by creating
`BlockedBy` relations on child tasks; `dependencies` is the read projection of those relations, not
a stored scalar field. Disjoint ready children may be shipped concurrently within the workspace's
normal run and lock limits. Dependency inference that exists only in a model's reasoning is
incomplete — the resident must create the relations before dispatch.

The parent does not become `done` merely because all children reached `review`. Completion requires:

- every required child is `done`, `rejected`, or explicitly accepted as out of scope;
- no open blocking review thread remains;
- every required PR has the human review/merge approval evidence required by workspace policy;
- required PRs are merged and candidate commits are on the integration branch;
- parent-level integration checks pass; and
- the parent execution summary contains a structured final verdict and evidence pointers.

## 8. Phasing Out the HTTP Epic Pipeline

`task_epic_pipeline` and its `epic_orchestrator` activity were removed as unused in [ORB-10332];
they were never converted in place because their contract differed materially from the resident
path:

| Former HTTP epic path | Resident CLI path |
|----------------|-------------------|
| Required pre-existing children | Owns decomposition and child creation |
| Used `backend: http` and a retained `session:` loop | Uses bounded `backend: cli` processes with an optional resumable provider conversation |
| Kept progress partly in provider conversation state | Keeps correctness in Orbit; conversation continuity only reduces reorientation cost |
| Dispatched child gates | Shepherds dispatch, review, merge, and closure |
| Treated `review` as shipped/terminal | Requires verified task and integration completion |

Because [ORB-10332] already removed the legacy epic-pipeline assets, the resident path can be built
greenfield without the earlier planned disjoint-selector migration:

1. ship the resident selector, resumable CLI activity contract, decision exchange, cycle job, and
   disabled routine;
2. canary one workspace and verify question/answer resumption, lost-session recovery,
   duplicate-dispatch prevention, review repair, and final parent closure; and
3. enable the routine per workspace as the resident capability proves out.

Historical run records for the removed pipeline remain readable after that catalog retirement.

## 9. Security and Authority

`backend: cli` delegates tool enforcement to the provider harness, so the resident activity is a
trusted workspace capability. The routine therefore requires the same review boundary as any
versioned automation asset, and the CLI run must retain Orbit's managed run identity and audit
envelope.

The resident's authority is bounded by the source workspace and live repository instructions.
It may exercise routine lifecycle operations needed to shepherd already-authorized work, but it
must block and surface a precise question when completion needs new product authority or a material
scope expansion.

Conversation references are opaque continuity pointers, not credentials or authorization grants.
Every resumed cycle starts under Orbit's current run identity, sandbox, workspace claim, provider
binding, and repository instructions. Decision answers retain their recorded human or supervisor
provenance. Resuming an old conversation must never restore superseded permissions or bypass a
newer durable task constraint.

## 10. Concerns & Honest Limitations

- **Workspace ownership is singular in v1.** Multiple resident orchestrators in one workspace need
  an explicit routing or lease design; tags alone are not enough.
- **Polling adds latency.** A five-minute cadence is simple and robust but delays pickup, answered
  decisions, and resumption. Event triggers remain outside routines v1.
- **Conversation continuity is best effort.** A provider may expire, reject, or become unable to
  resume a session. Durable state prevents correctness loss, but a replacement conversation pays
  the full reorientation cost.
- **Every cycle still refreshes authority and facts.** Resuming a conversation does not eliminate
  the cost of rereading current task, run, review, and repository state.
- **There is no live intervention in v1.** A human cannot steer an in-flight turn. Urgent control
  uses the existing run cancellation boundary; ordinary guidance is applied on the next cycle.
- **Cross-workspace epics are not one graph.** Each workspace owns its own task tree; the front door
  must relate and observe multiple parent tasks when one product outcome spans codebases.
- **Provider harness trust is real.** CLI activities do not receive the HTTP backend's builtin tool
  allowlist enforcement. Host sandboxing and provider policy remain part of the security boundary.
- **A resident can still make poor decomposition choices.** Deterministic selection and durable
  state, explicit assumptions, and decision gates make those choices inspectable and recoverable;
  they do not eliminate judgment risk.
- **Structured comments are intentionally narrow.** One pending decision per epic keeps v1 simple,
  but it is not a general mailbox, chat system, or multi-party negotiation log.

## Task References

- **[ORB-10332]** — Removed the unused HTTP epic pipeline assets that this design supersedes.
- **[ORB-10775]** — Implementation epic (children ORB-10776–ORB-10782).
- **[ADR-0361]** — The `epic` tag is the sole resident pickup selector.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
