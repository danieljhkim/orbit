---
type: design
summary: "Panic-Site Inventory — 2026-05"
tags: ["auditability"]
---
# Panic-Site Inventory — 2026-05

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-09 ([T20260509-6])
**Author:** claude-opus-4-7

Baseline inventory for the panic-audit task ([T20260509-6]). Companion to
[`scripts/audit_panics.py`](../../../scripts/audit_panics.py); regenerate the
counts in this doc by running `scripts/audit_panics.sh --json`.

---

## 1. Why this doc exists

The task body cites ~1,864 panic-pattern call sites and frames them as a
risk to durable execution. That headline number conflates four very
different populations. This doc:

1. Establishes a four-bucket count so the audit target has a meaningful
   denominator.
2. Defines a five-label taxonomy for classifying each non-test panic site
   in execution-critical crates.
3. Lists every blast-radius site (one row per site) with its classification,
   so subsequent phases of the task have a checklist.

This is intentionally a *measurement* doc, not a fix-list. Phase 2/3 of the
plan in [T20260509-6] consume these rows.

---

## 2. The four buckets

Counts produced by `scripts/audit_panics.sh` against
`crates/*/src/**/*.rs` on 2026-05-09. Definitions:

- **total** — every match for
  `\.unwrap\(\) | \.expect\( | panic!\( | unreachable!\( | unimplemented!\( | todo!\(`.
- **in_cfg_test** — same, but restricted to lines the heuristic identifies
  as living in a `#[cfg(test)]` scope, an inner `#[test]` / `#[tokio::test]`
  function body, or a path matching `*_tests.rs`, `*/tests/*`,
  `*test_support*`, `*/fixtures/*`.
- **non_test** — `total − in_cfg_test`.
- **blast_radius** — non-test matches in the four execution-critical
  crates only: `orbit-engine`, `orbit-agent`, `orbit-tools`, `orbit-exec`.

| Crate | total | in_cfg_test | non_test | blast_radius |
|---|---:|---:|---:|---:|
| orbit-agent     |   22 |   13 |  9 |  9 |
| orbit-cli       |  488 |  486 |  2 |  0 |
| orbit-common    |  105 |   90 | 15 |  0 |
| orbit-core      |  661 |  659 |  2 |  0 |
| orbit-engine    |  296 |  273 | 23 | 23 |
| orbit-exec      |   67 |   67 |  0 |  0 |
| orbit-knowledge |   68 |   50 | 18 |  0 |
| orbit-mcp       |    8 |    8 |  0 |  0 |
| orbit-policy    |   13 |   13 |  0 |  0 |
| orbit-registry  |    0 |    0 |  0 |  0 |
| orbit-store     |  177 |  175 |  2 |  0 |
| orbit-tools     |   19 |   15 |  4 |  4 |
| **TOTAL**       | **1924** | **1849** | **75** | **36** |

The classifier is a heuristic — it uses path conventions plus a search for
the last column-zero `#[cfg(test)] mod <name> { … }` and any
`#[test]` / `#[tokio::test]` function body. It does not handle strings that
span multiple lines, `r#"…"#` raw strings inside if-attributes, or an
inner module declaration in a separate file referenced via `mod foo;`. The
per-site rows in §4 were spot-checked against the source and remain
authoritative if a number drifts.

The remaining `non_test` count outside the four execution-critical crates
(39 sites across `orbit-cli`, `orbit-common`, `orbit-core`, `orbit-knowledge`,
`orbit-store`) is intentionally left out of scope for this task; the task
body explicitly deprioritizes them ("CLI entry points, test code, one-shot
setup paths").

---

## 3. Classification taxonomy

Each non-test execution-critical site below is labelled with exactly one
of the following:

- **`lock-poison`** — the panic is `Mutex::lock().expect(...)` or
  `Condvar::wait(...).expect(...)`. A poisoned lock means another thread
  already panicked while holding the guard, so the protected state is
  inconsistent. Returning `Err(...)` instead does not recover anything;
  it just changes the call shape. Phase 3 leaves these as-is and adds an
  invariant comment.

- **`infallible-by-construction`** — the panic is justified because the
  immediately-preceding code constructs the value being unwrapped, so the
  failure mode is unreachable unless the macro/library contract changes.
  Example: `serde_json::to_string(&value)` on a `Value::Object(...)` we
  just built, or `value.as_object_mut()` immediately after `json!({...})`.

- **`documented-invariant`** — the panic is sound but the soundness
  argument is not local; it depends on an upstream contract or an
  enforced state machine. These get a `// Invariant: …` line in Phase 3
  rather than a typed-error rewrite.

- **`accidental-should-return-result`** — the panic *is* an actual bug:
  the failure mode is genuinely reachable (parse failure, I/O failure,
  truly fallible serialization, …) and the enclosing function either
  already returns `Result` or could be changed to. Phase 2 converts these.

- **`test-only`** — non-test classification flag was wrong; the site
  belongs in a test block. None of the rows in §4 land here, but the
  label is included for completeness because regenerating the inventory
  may surface false positives that need re-labelling rather than fixing.

---

## 4. Blast-radius sites (36)

One row per non-test site in `orbit-engine`, `orbit-agent`, `orbit-tools`,
`orbit-exec`. The `class` column drives Phase 2 vs Phase 3 routing:
`accidental-should-return-result` ⇒ Phase 2 fix; everything else ⇒ Phase 3
invariant comment.

### orbit-agent (9)

| File:line | Snippet | Class | Notes |
|---|---|---|---|
| `loop_engine/audit/mod.rs:144` | `self.events.lock().expect("audit mutex").clone()` | `lock-poison` | InMemorySink event vec; only test/observation surface, but lock semantics matter. |
| `loop_engine/audit/mod.rs:154` | `self.events.lock().expect("audit mutex").push(...)` | `lock-poison` | Same mutex; emit path. |
| `loop_engine/audit/mod.rs:163` | `.lock().expect("blob mutex")` | `lock-poison` | InMemorySink blob vec. |
| `loop_engine/audit/mod.rs:218` | `Ok(writer.as_mut().expect("writer initialized"))` | `infallible-by-construction` | `ensure_writer` calls `writer.replace(...)` immediately above; `.as_mut()` is sound. |
| `loop_engine/audit/mod.rs:231` | `let mut writer = self.writer.lock().expect("audit writer");` | `lock-poison` | JsonlFileSink writer mutex. |
| `loop_engine/replay_transport.rs:80` | `let turns = self.turns.lock().expect("replay turns mutex");` | `lock-poison` | |
| `loop_engine/replay_transport.rs:81` | `let mut cursor = self.cursor.lock().expect("replay cursor mutex");` | `lock-poison` | |
| `loop_engine/tool_dispatch.rs:33` | `property.as_object_mut().expect("parameter schema")` | `infallible-by-construction` | `schema_for_param_type` returns `json!({...})`, always an Object. |
| `loop_engine/tool_dispatch.rs:50` | `input_schema.as_object_mut().expect("object")` | `infallible-by-construction` | `input_schema` is built from `json!({...})` two lines above. |

### orbit-engine (23)

| File:line | Snippet | Class | Notes |
|---|---|---|---|
| `activity_job/agent_loop_driver.rs:198` | `cell.lock().expect("replay mutex poisoned")` | `lock-poison` | Process-global REPLAY_TRANSPORT cache. |
| `activity_job/agent_loop_driver.rs:211` | `*cell.lock().expect("replay mutex poisoned") = None;` | `lock-poison` | Same cache; reset path. |
| `activity_job/cli_runner/supervisor.rs:174` | `buf.lock().expect("subprocess output buf poisoned")` | `lock-poison` | Stdout/stderr accumulation buffer. |
| `activity_job/groundhog.rs:845` | `.lock().expect("groundhog attempt mutex poisoned")` | `lock-poison` | |
| `activity_job/groundhog.rs:853` | `.lock().expect("groundhog attempt mutex poisoned")` | `lock-poison` | |
| `activity_job/groundhog.rs:859` | `self.state.lock().expect("groundhog attempt mutex poisoned")` | `lock-poison` | |
| `activity_job/groundhog.rs:877` | `self.state.lock().expect("groundhog attempt mutex poisoned")` | `lock-poison` | |
| `activity_job/job_executor/concurrency.rs:17` | `self.state.lock().expect("sem poisoned")` | `lock-poison` | Semaphore mutex. |
| `activity_job/job_executor/concurrency.rs:19` | `self.cond.wait(guard).expect("sem poisoned")` | `lock-poison` | Condvar wait propagates poison. |
| `activity_job/job_executor/concurrency.rs:28` | `self.state.lock().expect("sem poisoned")` | `lock-poison` | |
| `activity_job/job_executor/exec_ctx.rs:27` | `self.pipeline.lock().expect("pipeline poisoned").clone()` | `lock-poison` | |
| `activity_job/job_executor/exec_ctx.rs:79` | `.lock().expect("pipeline poisoned")` | `lock-poison` | |
| `activity_job/job_executor/fan_out.rs:59` | `ctx.pipeline.lock().expect("pipeline poisoned").clone()` | `lock-poison` | Fan-out call site cited in task body. |
| `activity_job/job_executor/fan_out.rs:69` | `.lock().expect("results poisoned")` | `lock-poison` | |
| `activity_job/job_executor/fan_out.rs:117` | `.lock().expect("results poisoned")` | `lock-poison` | |
| `activity_job/job_executor/fan_out.rs:130` | `results.into_inner().expect("results poisoned")` | `lock-poison` | `Mutex::into_inner()` poison. |
| `activity_job/job_executor/mod.rs:150` | `.lock().expect("pipeline poisoned")` | `lock-poison` | |
| `activity_job/job_executor/parallel.rs:46` | `h.join().expect("branch thread panicked")` | `documented-invariant` | Propagates branch-thread panic to parent on purpose; the alternative is silent loss. |
| `activity_job/job_executor/target.rs:39` | `ctx.sessions.lock().expect("sessions poisoned")` | `lock-poison` | |
| `activity_job/jsonl_sink.rs:56` | `self.writer.lock().expect("v2 jsonl writer mutex")` | `lock-poison` | V2 audit writer; durability surface. |
| `activity_job/tool_enforcement.rs:51` | `self.tripped.lock().expect("tripped mutex").clone()` | `lock-poison` | |
| `activity_job/tool_enforcement.rs:77` | `*self.tripped.lock().expect("tripped mutex") = ...` | `lock-poison` | |
| `activity_job/tool_enforcement.rs:101` | `*self.tripped.lock().expect("tripped mutex") = ...` | `lock-poison` | |

### orbit-tools (4)

| File:line | Snippet | Class | Notes |
|---|---|---|---|
| `builtin/orbit/duel/plan_winner.rs:41` | `serde_json::to_string(&winner_payload).expect("winner payload serializes")` | `accidental-should-return-result` | `winner_payload` contains user-supplied strings; serialization can theoretically fail (e.g. non-utf8 path strings). The enclosing function already returns `Result<Value, OrbitError>` — Phase 2 converts. |
| `builtin/orbit/knowledge/write.rs:143` | `Selector::Dir { .. } => unreachable!()` | `documented-invariant` | A `Selector::Dir` is filtered out earlier in the same function before this match runs; needs a comment but remains an `unreachable!`. |
| `graph.rs:240` | `value.as_object_mut().expect("node context payload object")` | `infallible-by-construction` | `value = json!({...})` two lines above. |
| `graph.rs:245` | `value.as_object_mut().expect("node context payload object")` | `infallible-by-construction` | Same `value` from line 231. |

### orbit-exec (0)

All non-test panic sites are confined to the test module (`#[cfg(test)] mod
tests { … }`). No blast-radius rows.

---

## 5. Class summary

| Class | Count | Phase |
|---|---:|---|
| `lock-poison`                       | 28 | Phase 3 (invariant comment) |
| `infallible-by-construction`        |  5 | Phase 3 (invariant comment, optional) |
| `documented-invariant`              |  2 | Phase 3 (invariant comment) |
| `accidental-should-return-result`   |  1 | Phase 2 (convert to `?`) |
| `test-only`                         |  0 | — |
| **TOTAL**                           | **36** | — |

`lock-poison` dominates by an order of magnitude. Per the plan's risk
section, converting these does not actually buy durability — a poisoned
mutex means another thread already panicked while holding the guard, so
the protected state is inconsistent regardless of whether we panic or
return `Err`. Phase 3 leaves them as `.expect(...)` with an explicit
invariant comment instead of pretending typed errors give us recovery we
cannot actually provide.

---

## 6. Acceptance-criteria mapping

This doc satisfies AC#1 of [T20260509-6] ("Inventory of all
.unwrap/.expect/panic! sites, classified into categories"). Subsequent
phases consume §4:

- AC#2 ("execution-critical paths converted") → Phase 2 changes the one
  `accidental-should-return-result` row.
- AC#3 ("intentional panics carry rationale") → Phase 3 adds invariant
  comments to the 35 non-accidental rows.
- AC#5 ("net panic-site count materially reduced") → measured against the
  36-row blast-radius bucket, not the 1,924 headline. Target: blast_radius
  count after Phases 2-3 ≤ 10 *uncommented* sites. Every commented site
  carries an invariant rationale.

---

## 7. How to regenerate

```sh
# Refreshed counts only:
scripts/audit_panics.sh

# Counts + JSON dump + blast-radius site list:
scripts/audit_panics.sh --json --sites > /tmp/audit_panics.json

# Single-line JSON for CI consumption:
scripts/audit_panics.sh --json | jq '.totals'
```

When the blast-radius count moves, refresh §2 and §4 in the same commit
that introduced the change so the inventory does not drift.
