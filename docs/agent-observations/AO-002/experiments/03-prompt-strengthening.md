---
parent: ../observation.md
recorded: 2026-05-18
lever: PLANNING_DUEL_INSTRUCTION rubric (per-run prompt, not memory)
status: refuted
artifact: commit 38f6c675 on agent-main
---

# Experiment 03 — PLANNING_DUEL_INSTRUCTION strengthening

## Setup

Moved the discovery rubric out of `GEMINI.md` (memory) and into the per-run `PLANNING_DUEL_INSTRUCTION` system/user prompt at [roles.rs:22](../../../../crates/orbit-engine/src/executor/automation/duel/planning_duel/roles.rs:22). Committed as `38f6c675` on `agent-main`.

Step 3 of the planner instruction now mandates `graph.refs` per moved symbol and `graph.search` per module boundary; `fs.read` is demoted to last-resort. Step 4 introduces MUST / MUST NOT clauses requiring named identifiers, enumerated consumers, exact verification commands (`cargo build -p <crate>`, `make ci-fast`, `rg`, `curl`), and banning hedge phrasing ("this should just work", "verify it compiles").

The two rubric-test runs (ORB-00151, ORB-00153) ran on `gemini-2.5-pro`. The pre-rubric baseline runs cited later in this experiment (ORB-00146, ORB-00149) ran on `gemini-3.1-pro-preview`. The before/after comparison therefore crosses a model-version boundary; what this experiment cleanly tests is rubric-effect on `gemini-2.5-pro`, with the 3.1-pro-preview pre-rubric runs serving as a same-family qualitative baseline rather than a controlled comparison.

## Hypothesis under test

Per-run prompt steering works where memory-file steering doesn't. If true, the next Gemini-as-planner session log shows:

- non-zero `orbit.graph.refs` calls,
- non-zero `orbit.graph.search` calls,
- the plan body names consumers and verification commands.

If false (same `graph.pack ×1 → read_file` profile), the gap is a hard model prior and the lever moves on to model-pinning — see [04-model-pin-stable](04-model-pin-stable.md).

## Result

**Refuted** on the implementation / refactor / audit axis. Per-run prompt steering can shape Gemini's *output*, but does not change its *tool selection*.

Two Gemini-as-planner duels ran against commit `38f6c675` — both on `gemini-2.5-pro`:

| run | task type | model | outcome | Gemini tool profile |
|---|---|---|---|---|
| ORB-00151 (jrun-20260518-0528) | website audit | gemini-2.5-pro | lost to Codex | `graph.pack ×1`, `read_file ×3`, `web_fetch ×1` |
| ORB-00153 (jrun-20260518-0559) | JS module split | gemini-2.5-pro | lost to Claude | `read_file ×2`, `write_file ×1` — **zero graph calls of any kind** |

Wall clock on ORB-00153: planner_b (Gemini) finished in 54s vs planner_a (Claude) at 300s — Gemini was 5.5× faster, consistent with not investigating the codebase before drafting.

### What landed from the rubric

- **Verification commands.** ORB-00153's plan includes `node --check`, `cargo build -p orbit-dashboard`, `make ci-fast`, and a manual UI checklist. Compare to ORB-00146 (pre-rubric): zero commands.
- **Risk specificity.** ORB-00153's risks name actual failure modes (state partitioning, circular ES module imports) rather than generic boilerplate.
- **Structural shape.** Numbered steps, separate Risks / Verification sections, severity-tagged findings on ORB-00151 (~30 entries, location-prefixed format).

### What did not land

- **`graph.refs` per moved symbol.** 0 calls in both runs. Rubric's central clause ignored.
- **`graph.search` per module boundary.** 0 calls in both runs.
- **Required `*authored by:*` signature.** Absent in ORB-00153 — an explicit, mechanical per-run instruction ignored.
- **Hedge phrasing.** ORB-00151 still contains "seems to match", "I cannot directly measure", "I cannot fully test", "might reveal" — MUST-NOT clause violated 5+ times.
- **Hallucinated symbol names.** ORB-00153 cites `handleAuditHashChange` and `initAuditFilter` as entry points to import from `audit.js`. Neither exists in `app.js`. Exactly the failure mode `graph.refs` is designed to prevent.

### Causal link

The cost of skipping `graph.refs` materialized directly in the arbiter rationale for ORB-00153 (Codex as arbiter):

> "Planner B ... proposes underspecified or nonexistent entry points such as `handleAuditHashChange`/`initAuditFilter`, making it less aligned with the codebase."

A single `graph.refs` on `setActiveTab` or a `graph.search` for `handleAudit*` would have surfaced the absence in seconds. The rubric required the call; Gemini skipped it; the missing call produced fabricated symbols; the fabrication lost the duel.

### Pattern across all four experiment runs

A consistent shape now spans ORB-00146 (no rubric, gemini-3.1-pro-preview), ORB-00149 (`GEMINI.md` rubric, gemini-3.1-pro-preview), ORB-00151, and ORB-00153 (`PLANNING_DUEL_INSTRUCTION` rubric, both on gemini-2.5-pro):

| layer | shifted by rubric? |
|---|---|
| internal deliberation (thoughts size) | **yes** — ORB-00149 thoughts ballooned 2.5× under the GEMINI.md rubric |
| output structure (sections, format, severities) | **yes** — verification sections, location-prefixed findings now appear |
| output content quality (named identifiers, command specificity) | **partial** — names get lifted from the task description or fabricated, not graph-discovered |
| tool selection | **no** — same `graph.pack ×1 → read_file` pattern, sometimes fewer graph calls than baseline |

Instruction surface (file or prompt) can shape what Gemini *writes*. It cannot shape what Gemini *calls*.

## Scope of this verdict

Refuted for **implementation, refactor, and audit-shaped tasks on `gemini-2.5-pro`** — that's the configuration the rubric was actually tested against (ORB-00151 audit, ORB-00153 module split). The two earlier baseline data points (ORB-00146, ORB-00149) ran on `gemini-3.1-pro-preview` pre-rubric and serve as qualitative context only; the experiment did not run the rubric against `gemini-3.1-pro-preview` on an implementation-shaped task, so the rubric's effect on 3.1-pro-preview specifically remains untested on this axis.

UX / design-taste tasks are a separate axis (see [04-ux-taste-axis](04-ux-taste-axis.md)) and are not part of this verdict.

## Reproducing

```sh
git show 38f6c675 -- crates/orbit-engine/src/executor/automation/duel/planning_duel/roles.rs

# Per-run artifacts:
cat ~/.orbit/tasks/workspaces/orbit-*/ORB-00151/artifacts/files/planning-duel/planner_a.md
cat ~/.orbit/tasks/workspaces/orbit-*/ORB-00153/artifacts/files/planning-duel/planner_b.md

# Tool-call counts per Gemini session:
jq -r 'select(.toolCalls != null) | .toolCalls[]?.name' \
  ~/.gemini/tmp/orbit/chats/session-2026-05-18T05-28-30c40307.jsonl \
  | sort | uniq -c | sort -rn
jq -r 'select(.toolCalls != null) | .toolCalls[]?.name' \
  ~/.gemini/tmp/orbit/chats/session-2026-05-18T05-59-2a5537c5.jsonl \
  | sort | uniq -c | sort -rn

# Scoreboard rows:
jq '.runs[] | select(.task_id == "ORB-00151" or .task_id == "ORB-00153")' \
  .orbit/state/scoreboard/duel_plan.json
```
