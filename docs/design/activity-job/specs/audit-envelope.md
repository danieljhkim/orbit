# Spec: V2 Audit Envelope

Activity / Job runs emit a structured v2 audit tree that describes run, step, activity, and control-flow structure. This tree is append-only JSONL and coexists with the lower-level loop transcript/blob sink rather than replacing it.

## Why This Exists

The lower-level loop audit is rich, but it does not describe job structure on its own. Reviewers need to answer questions like:

- which job run emitted this activity?
- which step retried?
- which branch failed a join?
- which workspace produced this run?

The v2 envelope adds that structure.

## Event Tree

Every event carries:

- `schemaVersion`
- `event_type`
- `event_id`
- timestamp
- `run_id`
- `agent_identity`
- optional `parent_event_id`
- optional `workspace_path`

Common event families are:

- `run.*`
- `step.*`
- `activity.*`
- construct-level events for `parallel`, `fan_out`, and `loop`
- policy/tool denial and CLI invocation events

`cli.invocation.started` includes redacted argv, stdin blob ref, optional model, wall-clock timeout, and optional `cwd`. When present, `cwd` is the subprocess working directory selected by Activity/Job's workspace resolver.

Loop-engine HTTP and tool-call events remain in the lower-level sink and are related to the envelope tree by parentage and shared run identity.

## Persistence Layout

Envelope events append to:

```text
.orbit/state/audit/v2_loop/<run_id>.jsonl
```

Loop-engine events and blobs continue to use the sibling audit layout under:

```text
.orbit/state/audit/loop/<run_id>.jsonl      created on first loop event
.orbit/state/audit/blobs/<hh>/<hash>
```

The v2 writer may also keep an in-memory snapshot for smoke assertions and CLI summaries.

## CLI Inspection

`orbit run events [run_id]` is the human-facing chronological reader for the v2 envelope. It resolves the same default run as `orbit run show`, prints timestamp, derived activity `step.id`, event type, and a concise body summary, and can emit the flattened raw event objects as JSON with derived `step_id` attached to descendant events.

`orbit run trace [run_id]` renders the `event_id` / `parent_event_id` tree. JSON mode returns deterministic `roots` and `orphans` arrays so partial traces remain inspectable instead of silently dropping events whose parent is absent.

`orbit run logs` reads CLI stdout/stderr through the same runtime-owned envelope accessor rather than parsing the file layout in the CLI command module. `orbit run show -s <id>` treats the activity DAG `step.id` from `step.started` events as the primary step identifier, with legacy job-run target IDs and numeric step indexes retained as fallbacks. This inspection surface landed.

## Invariants

- Envelope writes are append-only, one JSON object per line.
- Disk persistence failure should not crash the run by itself; the in-memory event stream is still load-bearing.
- `workspace_path` is attached when the caller has a meaningful repo identity.
- CLI invocation start events include `cwd` when the runtime selected a subprocess working directory.
- Parent stacks propagate into worker threads so nested branch/worker events remain traversable.

## Failure Modes

- Audit writer mutex poisoning surfaces as a structured audit failure.
- JSONL persistence can fail independently of in-memory event capture.
- Reviewers may need both the envelope JSONL and the lower-level loop JSONL/blob store to reconstruct a full run.

## Migration Notes

- The envelope is additive. It does not retire or rewrite the existing loop-level audit sink.
- CLI backend events are first-class envelope events, so CLI runs remain visible even when no HTTP transcript exists.
- File-backed runtime traces moved from `.orbit/audit/` to `.orbit/state/audit/`. Existing `.orbit/audit/` files are legacy local artifacts rather than the current write target.

## Agent Signature

Last revised by `codex` for [T20260508-8].
