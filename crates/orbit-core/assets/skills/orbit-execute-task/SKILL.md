---
name: orbit-execute-task
description: Use this when executing an existing Orbit task or carrying a human request through the Orbit task lifecycle with explicit status tracking.
---

# Orbit Execute Task

## Purpose

Handle a human-requested engineering task or existing Orbit task from intent to verified implementation, with explicit task lifecycle tracking.

## Command Reference

Orbit task tools are available via two surfaces; both accept identical JSON. **Always include `model` in the JSON args.** The value is your agent family (`codex`, `claude`, `gemini`, or `grok`); full model strings are accepted and auto-normalized, but the family is canonical.

- **MCP** (plugin path): call `orbit_task_show`, `orbit_task_start`, `orbit_task_update`, `orbit_task_list` directly.
- **CLI**: `orbit tool run orbit.task.<action> --input '<json>'` — never use `orbit task ...` directly, it skips agent provenance.

Mapping rule: `orbit.<group>.<action>` ↔ `orbit_<group>_<action>`. See the `orbit` skill for full coverage. Never guess tool names — run `orbit tool list` (CLI) or `tools/list` (MCP) to see all registered tools.

```json
{ "model": "<agent-family>" }
```

CLI examples (substitute the MCP form using the mapping above):

```bash
# Load a full task (MCP: omit `field`/`fields` → returns full task)
orbit tool run orbit.task.show --full --input '{"id": "<task-id>", "model": "<agent-family>"}'

# Load a specific field only
orbit tool run orbit.task.show --input '{"id": "<task-id>", "field": "plan", "model": "<agent-family>"}'
# Valid fields: comments, plan, execution_summary, description, acceptance_criteria, history, context_files, artifacts

# Start a task (proposed/backlog/someday/blocked -> in-progress)
orbit tool run orbit.task.start --input '{"id": "<task-id>", "note": "<why>", "model": "<agent-family>"}'

# Update plan or add a comment
orbit tool run orbit.task.update --input '{"id": "<task-id>", "plan": "<markdown plan>", "model": "<agent-family>"}'
orbit tool run orbit.task.update --input '{"id": "<task-id>", "comment": "<what happened>", "model": "<agent-family>"}'

# Persist execution summary
orbit tool run orbit.task.update --input '{"id": "<task-id>", "execution_summary": "<summary>", "model": "<agent-family>"}'

# List tasks
orbit tool run orbit.task.list --input '{"status": "backlog", "model": "<agent-family>"}'
```

## Workflow

### Step 1: Load or create the task

**If given an existing task ID**, load it with `orbit.task.show`. Extract:
- `description` and `acceptance_criteria` — these define the required outcome.
- `plan` — if blank or placeholder, author a plan before starting.
- `context_files` — treat these as selectors first. Prefer `file:`, `dir:`, or `symbol:` forms, use `orbit.graph.pack` when available, and fall back to `fs.read` only for unresolved selectors.
- `status` — confirm the task is ready to start.

Then, if `orbit.search` is available, call it with `semantic: "<task-id>"` and `limit: 5`. Rationale: surface prior tasks the original author may not have linked via `context_files` — past decisions, prior attempts at the same problem, related review threads. Skim snippets; usually one hit is genuinely useful and the rest are noise. **This step is non-blocking** — if the companion binary is missing (install-pointer error) or no hit is relevant, continue. See `orbit-search`.

**If this is a new task** (no task ID), clarify intent and success criteria with the human, then create via `orbit-create-task`.

### Step 2: Plan

If the task lacks a concrete plan, write one:

```bash
orbit tool run orbit.task.update --input '{"id": "<task-id>", "plan": "<markdown plan>", "model": "<agent-family>"}'
```

Replace placeholders like `To be authored by executing agent at start time.` Keep the plan concrete: target files, validation commands, risks.

### Step 3: Start

```bash
orbit tool run orbit.task.start --input '{"id": "<task-id>", "note": "<why this is ready>", "model": "<agent-family>"}'
```

- Moves `backlog -> in-progress` or `proposed -> in-progress` (records approval automatically).
- Starting from `proposed` still requires a real plan; starting from `backlog` does not.
- Use explicit `approve` + later status updates when approval and execution should stay separate.

### Step 4: Implement and validate

Follow the task's `plan` step by step. Use selector-first context from `context_files` before touching code: prefer `orbit.graph.pack`, and read files directly only for unresolved selectors or when the graph is unavailable. Run the repo-approved verification commands from the plan. If repo instructions forbid tests, honor that and use the allowed validation path instead.

### Step 5: Summarize and hand off

First persist the execution summary (see template below):

```bash
orbit tool run orbit.task.update --input '{"id": "<task-id>", "execution_summary": "<summary>", "model": "<agent-family>"}'
```

Learning checkpoint: after persisting the summary, consider whether the task
surfaced a contradicted assumption, recurring failure mode, non-obvious gotcha
that took more than 10 minutes to debug, or incident-style root cause. If so,
follow the `orbit-learning` skill and call `orbit.learning.add` (or use that
skill's update/supersede flow when it points to existing guidance). Skip if
none apply.

Then choose the lifecycle handoff path:

- **Under an activity envelope** (for example, `agent_implement`): persist the
  `execution_summary` only. Do not move the task to `review`; the pipeline owns
  that transition after commit/merge or PR steps succeed. If the envelope gives
  lifecycle instructions, it takes precedence over this skill's direct-execution
  defaults.
- **Direct execution** (no activity envelope, ad-hoc human-requested work):
  persist the `execution_summary` and move the task to `review` via
  `orbit.task.update`.

```bash
orbit tool run orbit.task.update --input '{"id": "<task-id>", "status": "review", "model": "<agent-family>"}'
```

## Execution Summary Template

The generated PR body supplies the top-level `## Task`, `## Execution Summary`,
`## Validation`, and `## Branch Freshness` sections. Keep the persisted
`execution_summary` focused on what changed; it will be rendered inside the
collapsed `## Execution Summary` block, so do not duplicate PR body section
headings in the summary.

Required content:

```markdown
Outcome: success | failed

Changes:
- <what changed and why>

Assessment: <short quality assessment>
```

Include when relevant (omit if N/A):

```markdown
Strategic decisions:
- <decision> | Rationale: <why>

Design weaknesses / risks:
- <risk> | Severity: Low / Medium / High | Mitigation: <mitigation>

Deviations from original plan:
- <deviation> | Justification: <why>

Recommended follow-ups:
- <next step>
```

## Lifecycle Rules

- One Orbit task per activity invocation. Do not multiplex tasks.
- If material ambiguity remains, ask clarifying questions before implementation.
- If approval cannot be obtained for `proposed` work, stop after recording that state.
- Do not skip lifecycle updates.
- Direct execution must persist a non-empty `execution_summary` before or
  together with the review transition. Envelope-driven execution persists the
  summary and leaves the review transition to the owning pipeline.

## Exit Criteria

- Requested change implemented and validated.
- Task started via `orbit.task.start` before execution.
- Execution summary persisted via `orbit.task.update`.
- Learning checkpoint considered; if a load-bearing insight was identified,
  `orbit.learning.add` was called per the `orbit-learning` skill.
- For direct execution, task advanced to `review`.
- For envelope-driven execution, task left for the pipeline-owned review
  transition after commit/merge or PR steps succeed.
