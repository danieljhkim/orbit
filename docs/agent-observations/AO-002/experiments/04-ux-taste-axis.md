---
parent: ../observation.md
recorded: 2026-05-18
lever: Task-shape rotated to UX / design-taste (from implementation / refactor / audit)
status: inconclusive
artifact: ORB-00154 + ORB-00155 (paired duels on identical task)
---

# Experiment 04 — UX-taste axis, cross-model comparison

## Setup

The implementation-axis verdict from experiment 03 left a question explicitly open: does Gemini's standing change when the task type rotates away from refactor / audit / module-split and toward pure design judgment?

To answer it with all four model families on **identical input**, the same task was queued twice as disjoint planner pairs:

- **ORB-00154** (jrun-20260518-0603) — planner_a = codex, planner_b = claude, arbiter = gemini. **Winner: claude.**
- **ORB-00155** (jrun-20260518-0620) — planner_a = grok, planner_b = gemini, arbiter = codex. **Winner: grok.**

Task content: replace the six stacked scoreboard tables in the Orbit dashboard with one comparable layout, with at-a-glance cross-agent comparison, no-zero-glyph rendering, magnitude encoding, single-screen fit at 1440×900, and a new ADR. Pure design judgment plus execution quality; no symbol migration of significance. Full task description + acceptance criteria preserved at [../references/task.md](../references/task.md).

The four plans the analysis below works from are each preserved verbatim under [`../references/`](../references/):

- [plan-codex.md](../references/plan-codex.md) — ORB-00154 planner_a, lost to claude
- [plan-claude.md](../references/plan-claude.md) — ORB-00154 planner_b, won
- [plan-grok.md](../references/plan-grok.md) — ORB-00155 planner_a, won
- [plan-gemini.md](../references/plan-gemini.md) — ORB-00155 planner_b, lost to grok

Rubric unchanged from experiment 03 (commit `38f6c675`). ORB-00155 ran on `gemini-3.1-pro-preview`; ORB-00153 ran on `gemini-2.5-pro`. The comparison between ORB-00153 (refactor) and ORB-00155 (UX redesign) therefore crosses both a task-shape boundary and a model-version boundary (2.5-pro → 3.1-pro-preview). Two variables moved; we cannot attribute the Gemini tool-profile shift in ORB-00155 to task-shape alone.

## Hypothesis under test

Question, not hypothesis:

1. When the task type rotates to UX/design-taste, does Gemini's **tool profile** change?
2. Does Gemini's **plan content** reach parity with the other three families on design-taste signals (layout choice, visual encoding, ADR depth)?
3. Does Gemini's standing in the duel outcome change?

## Result

**Inconclusive.** All four plans and both arbiter rationales in hand. The reading below is decision-grade-for-us, not an objective ranking — see Caveats. Inconclusive because the rotated-task run (ORB-00155) crossed both a task-shape and a model-version boundary; we cannot cleanly attribute Gemini's tool-profile shift to one or the other. The standing question is answered (Gemini ranked last on plan depth in the 4-way comparison too); the tool-engagement attribution is not.

### Gemini tool-profile shifted

| run | task shape | model | tool profile |
|---|---|---|---|
| ORB-00146 | refactor | gemini-3.1-pro-preview | `graph.pack ×1`, `read_file ×4` |
| ORB-00149 | module split | gemini-3.1-pro-preview | `graph.pack ×1`, `read_file ×1`, `write_file ×3`, `run_shell_command ×2` |
| ORB-00151 | site audit | gemini-2.5-pro | `graph.pack ×1`, `read_file ×3`, `web_fetch ×1` |
| ORB-00153 | module split | gemini-2.5-pro | (no graph calls), `read_file ×2`, `write_file ×1` |
| **ORB-00155** | **UX redesign** | **gemini-3.1-pro-preview** | **`graph.refs ×4`, `graph.pack ×2`, `graph.search ×1`**, `grep_search ×1`, `run_shell_command ×2`, `write_file ×1` |

First Gemini-as-planner session with non-zero `graph.refs` or `graph.search`. Plan opens with `## 1. Hidden Coupling / Context` — mirrors the rubric structure verbatim. The `*authored by: gemini / planner_b*` signature is also back after going missing on ORB-00153.

Caveat (two-variable shift): n=1 on this axis, and ORB-00155 changed *both* task shape (refactor → UX redesign) *and* model version (gemini-2.5-pro → gemini-3.1-pro-preview) relative to ORB-00153. We cannot separate "rubric works on 3.1-pro-preview but not on 2.5-pro" from "graph engagement is task-shape-dependent for Gemini." Either reading is consistent with the data. The 3.1-pro-preview baseline runs (ORB-00146 / 00149, both pre-rubric, both refactor-shaped) also showed no `graph.refs` / `graph.search` calls, which weakens but does not eliminate the model-version hypothesis.

### 4-way plan comparison

| signal | codex (00154/A) | claude (00154/B) | grok (00155/A) | gemini (00155/B) |
|---|---|---|---|---|
| outcome | lost | **won** | **won** | lost |
| lines | 84 | 233 | 94 | 57 |
| layout choice | agent-major + grouped cols | **metric-major** (rows=metrics) | agent-major flat (12 cols) | agent-major + colspan groups |
| sortable headers | yes (overall + 5 metrics) | **deliberate no** | yes | yes |
| zero glyph | `·` zero-dot | `—` em-dash | `–` en-dash | `·` w/ `color: transparent` |
| ADR | full body + 3 rejected alts | full body + 3 rejected alts + ID allocated + frontmatter bump | full body + 2 rejected alts | named file only, no body content |
| hidden-coupling enumeration | 7 bullets, all symbols named | 15+ items w/ line numbers + `rg` verification | 4 bullets w/ line numbers | 3 bullets |
| risks named | 4 (specific failure modes) | 7 (each w/ mitigation) | 3 | 1 |
| verification commands | 8 (incl. preview_resize / screenshot / inspect) | 6 + 7 browser sub-checks | 7 | 3 |

### UX-taste reading

- **Claude is the only one to flip the axis to metric-major** and the only one to drop sortable headers as a deliberate choice ("per-row leader marker makes 'who's winning this metric' instant"). Strongest design-call signal in the set; the other three all chose agent-major and leaned on sort affordances.
- **Codex** was the only one to break non-canonical agents into a separate "Attribution Cleanup" sub-matrix — functional, not necessarily the UX win.
- **Gemini's `color: transparent` on zero cells** is a clever subordinate detail — preserves cell width without visible glyph. Smallest design move in the set but real taste.
- **Grok** went 12 fixed columns at 1440px — risks horizontal density; Grok flagged this in its own risks.

### Execution quality reading

Ordering on detail, ADR completeness, risk depth: claude > codex > grok > gemini. Gemini's plan is the only one without actual ADR content (just names the file). One named risk vs the others' 3–7 is consistent with Gemini-as-planner thinness across all five runs.

### Arbiter rationale for ORB-00154

Gemini-as-arbiter chose Claude on (a) metric-major layout = better at-a-glance comparison, (b) thoroughness, (c) detailed risk analysis with mitigations, (d) precise verification commands. (a) is the pure UX-taste call; (b)/(c)/(d) are execution-quality clauses. The arbiter weighted both, not just taste.

### Arbiter rationale for ORB-00155

Codex-as-arbiter chose Grok on (a) compact sortable leaderboard, (b) new table class that avoids the shared `scoreboard-table` CSS used by diagnostics / audit / run events, (c) explicit zero-glyph suppression, (d) leader perf bars, (e) count labeling, (f) canonical / non-canonical row gating, (g) full ADR body, (h) browser/CI verification. Cited Gemini's losses: horizontal-overflow risk at 1440px, "duel matrix / cleanup grouping less resolved", "less specific about exact writeback path."

No design-taste differential was called out — both Grok and Gemini chose agent-major orientation, so the arbiter ranked on execution quality alone.

### Cross-arbiter notes

The two arbiters weighted differently:

- **Gemini-as-arbiter** (00154) rewarded a bold design call (metric-major) alongside execution quality.
- **Codex-as-arbiter** (00155) rewarded code-alignment, CSS-hazard awareness, and ADR / verification specifics. With no design-call differential in its pair, it ranked execution alone.

Worth flagging that Gemini-as-arbiter could recognize and reward design boldness — while Gemini-as-planner produced the most conventional plan in the set (agent-major + colspan groups, like Grok). Arbiter mode benefits from having the artifacts in front of it; planner mode has to imagine outputs. Different cognitive postures, same model family.

### Our cross-read

Both arbiters' rankings are consistent with this ordering for their respective pairs. The cross-pair ordering (e.g. Claude vs Grok) is our read, not arbiter-validated.

| rank | model | strengths | shortcomings |
|---|---|---|---|
| 1 | **claude** | bold design call (metric-major + deliberate no-sort); 233-line plan; 15+ hidden-coupling items w/ line numbers + `rg` verification; 7 risks w/ mitigations; full ADR allocation incl. ID + frontmatter bump; CSS class-reuse hazard explicitly handled | length itself can intimidate an implementer; "ties get all ▲" is debatable on dense rows; metric-major axis costs a vertical scan when an operator wants "how is Codex doing today" |
| 2 | **codex** | very thorough, methodical; metric-normalization called out explicitly; sortable headers with explicit key list; full ADR + 3 rejected alts; preview-tooling verification | agent-major is the conventional choice — no design risk taken; "overall sort" weighting flagged but not fully solved; vertical density at scale not addressed |
| 3 | **grok** | compact (94 lines) but specific; new CSS class avoids `scoreboard-table` reuse hazard (same awareness as Claude); concrete `LEADERBOARD_COLUMNS` inline w/ full metric descriptors; full ADR + last_updated bump + 2 rejected alts; 7 verification commands | 12 fixed cols at 1440px (Grok flagged in its own risks); inline-style JS rather than CSS class blurs the JS/CSS boundary; same conventional agent-major orientation as Codex/Gemini |
| 4 | **gemini** | first time engaging graph tools in any duel session (`graph.refs ×4`, `graph.search ×1`); clever `color: transparent` zero-glyph trick preserves cell width without visible glyph; hidden-coupling section opens the plan, mirroring the rubric | shortest plan (57 lines); ADR named but not written; 1 risk only; no realistic wire sketch with values; small internal contradiction (lists `renderScoreboardCell` to modify, but step 2 says remove it); no specific writeback or verification path |

### Tool engagement ≠ output quality

Gemini called graph tools for the first time but the resulting plan is the **shortest and lightest of the four** (57 lines vs 84/94/233). The hidden-coupling section is 3 bullets vs Claude's 15+. So graph engagement and plan depth are separate levers; engaging one does not pull the other.

## Caveats

- **n=1 on the UX axis.** One task tells us about a model's ceiling on one task, not its UX taste across the surface.
- **Gemini model version varied across the thread.** ORB-00155 ran on `gemini-3.1-pro-preview`; ORB-00151 and ORB-00153 (the implementation-axis rubric-test runs in experiment 03) ran on `gemini-2.5-pro`. Cross-experiment comparisons cross both task-shape and model-version boundaries.
- **4-way ordinal can't fall out of the two arbiter rationales alone.** Different pairs, different arbiter families. A clean ordinal across the four needs a separate cross-read (human or arbiter-on-all-four).
- **Same caveats as experiment 03 apply:** single-codebase context, arbiter judgment is evidence not ground truth, no within-model variance, task selection biased to what was on deck.

## Reproducing

```sh
# All four plans
cat ~/.orbit/tasks/workspaces/orbit-*/ORB-00154/artifacts/files/planning-duel/planner_a.md  # codex
cat ~/.orbit/tasks/workspaces/orbit-*/ORB-00154/artifacts/files/planning-duel/planner_b.md  # claude
cat ~/.orbit/tasks/workspaces/orbit-*/ORB-00155/artifacts/files/planning-duel/planner_a.md  # grok
cat ~/.orbit/tasks/workspaces/orbit-*/ORB-00155/artifacts/files/planning-duel/planner_b.md  # gemini

# Gemini tool profile + model confirmation
jq -r 'select(.toolCalls != null) | .toolCalls[]?.name' \
  ~/.gemini/tmp/orbit/chats/session-2026-05-18T06-20-c8efb70a.jsonl \
  | sort | uniq -c | sort -rn
jq -r 'select(.model != null) | .model' \
  ~/.gemini/tmp/orbit/chats/session-2026-05-18T06-20-c8efb70a.jsonl | sort -u

# Scoreboard rows + both arbiter rationales
jq '.runs[] | select(.task_id == "ORB-00154" or .task_id == "ORB-00155")' \
  .orbit/state/scoreboard/duel_plan.json

# Winner artifacts (arbiter outputs)
cat ~/.orbit/tasks/workspaces/orbit-*/ORB-00154/artifacts/files/planning-duel/winner.json
cat ~/.orbit/tasks/workspaces/orbit-*/ORB-00155/artifacts/files/planning-duel/winner.json
```
