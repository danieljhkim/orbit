# Authoring a task

Write a task another engineer or agent can execute without guessing: a crisp
problem statement plus acceptance criteria that define observable success. The
execution plan is authored later, at pickup, not here.

Every `orbit.task.*` call needs `model` — your agent family. Never use bare
`orbit task ...`; it skips agent provenance.

## Workflow

1. **Confirm** the objective, constraints, and what done means.
2. **Check for overlapping prior work.** Run a hybrid search on the title and
   description before creating anything — a brand-new task has no embeddings, so
   `--hybrid --kind task` on the text is the check that works. (`search similar`
   needs an existing task with vectors; it is for pickup, not creation.)
   → [search.md](search.md)
3. **Write acceptance criteria that name observable success** — a command, an
   inspection step, or an output. "Works correctly" is not a criterion.
4. **Optionally fill `context_files`** (see below).
5. **Set `complexity`** (`low` / `medium` / `hard`). It is required at
   creation. `unassessed` is reserved for automated mint/import and is not
   an operator create value.
6. **Add assumptions, risks, and rollback notes** to the description when they
   matter.
7. **Call `orbit.task.add`.** Confirm via the result, or re-fetch with
   `orbit.task.show`.

```bash
orbit tool run orbit.task.add --input '{
  "title": "<title>",
  "description": "<multi-line markdown>",
  "acceptance_criteria": ["<observable outcome>", "<observable outcome>"],
  "context_files": ["file:src/lib.rs", "dir:src/command"],
  "required_tools": ["<exact.canonical.tool>"],
  "workspace": "<selector>", "priority": "<low|medium|high|critical>",
  "complexity": "<low|medium|hard>", "type": "<feature|bug|refactor|chore>",
  "model": "<agent-family>"
}'
```

## `context_files`

Names *only* modification and deletion targets, as canonical selectors
(`file:`, `dir:`, `symbol:path#name:kind`), each resolving inside the target
workspace's root — an out-of-root path fails pipeline admission.

Read-for-context files, convention and pattern docs, and files that don't exist
yet do not belong there; cite those in prose instead. The exception is a design
doc the repo co-locates with the code it describes, since it co-changes.

Prefer `file:`/`symbol:` over `dir:` when the change can be named precisely.

The field is optional unless the workspace's own policy requires it. Leaving it
empty is valid, and **guessing entries to avoid an empty field is worse than
empty** — the list is what conflict detection reads, so a wrong entry actively
misleads. When an orchestrator needs selectors prepared at scale, the task-pilot
job fills them from real inspection. → [orchestration.md](orchestration.md)

## Operating rules

- Never edit task files directly; never invent task IDs (`orbit.task.add`
  allocates them).
- Required: `title`, `description`, `workspace`, `complexity`. Strongly prefer
  `acceptance_criteria`.
- `description` should be multi-line markdown for anything non-trivial.
- Valid `type`: `feature`, `bug`, `refactor`, `chore`.
- Do not pass the retired `plan` field.
- Blank companion files (`plan.md`, `execution-summary.md`) are blank *fields* —
  repair with `orbit.task.update`, never by hand.

## Behavior-affecting optional fields

- `dependencies: ["<task-id>", ...]` — prerequisites must reach a satisfying
  status first.
- `relations: [{"type": "resolves", "target": "<friction-id>"}]` — auto-resolves
  that friction when this task reaches `done`. Other types (`produces`,
  `blocked_by`, `child_of`, `spawned_from`, `regression_from`, `supersedes`,
  `related_to`) are tracked but inert. Only `produces`/`resolves` accept
  non-task targets; the rest require a task ID. A dangling target succeeds but
  emits a `TaskRelationDangling` audit event.
- `parent_id`, `source_task_id` (the bug-introducing task; creation-time only —
  `update` silently drops it), `tags` (reuse existing before inventing new).
- `required_tools: ["<exact.canonical.tool>", ...]` — tools the task must add to
  any agent activity's baseline. Use only exact, active, agent-facing registered
  names; wildcards and prefixes are rejected at dispatch. The list is sorted and
  deduplicated on write, and cannot change once the task enters `in-progress`.
  Inclusion grants only activity allowlist membership: caller role, host
  capability, tool policy, filesystem/subprocess policy, and external
  authentication can still deny execution.

## Quality bar

Validation must not assume uncommitted artifacts or workspace-local runtime
state under `.orbit/state/`. File I/O checks use temp dirs or fakes.
Behavior-changing work that touches external services, the filesystem, or time
should ask for deterministic mock coverage in its acceptance criteria.

## Description template

```markdown
## Problem
<what is broken, missing, or needs to change>
## Why It Matters
<user impact, operational impact, or engineering rationale>
## Constraints / Notes
- <important constraint>
```

Exit: the task exists with a strong description, clear acceptance criteria, and
— when filled — `context_files` naming only real modification targets.
