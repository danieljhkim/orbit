---
id: AO-002
title: Instruction surface shapes plan output, not tool selection
owner: daniel
last_updated: 2026-05-18
recorded: 2026-05-18
tags: [agent, instruction, performance]
---

# Instruction surface shapes plan output, not tool selection

**status:** closed (decision-grade-for-us; see Caveats for what this is not)

## Background

Gemini has been the lowest performer on plan duels. In the 30 days leading up to this observation, Gemini had yet to win a single duel against Claude Code, Codex, or Grok Build.

3 days ago, while checking the duel logs, I noticed that Gemini's model was set to "auto", which means a weaker model was being used. I have since switched to using "pro" and reset the scores to 0, but Gemini remained the lowest performer at 0/10 wins. (Per the Gemini CLI alias docs, `pro` resolves to `gemini-2.5-pro` *or* `gemini-3-pro-preview` depending on whether preview features are enabled. The actual model used varied across the experiments in this thread — ORB-00146/00149/00155 ran on `gemini-3.1-pro-preview`; ORB-00151/00153 ran on `gemini-2.5-pro`. Each experiment file records the model used for that run.)

On 2026-05-18 I learned that AGENTS.md is not auto-injected to Gemini's context — Gemini CLI reads `GEMINI.md`, not `AGENTS.md`. That was the working hypothesis for experiment 1, and the rest of the thread followed from how that hypothesis failed.

To reduce other biases at thread start:
- Removed root-level agent instructions for Claude, Codex, and Grok.
- Dropped Claude from `[duel.candidates]` in `.orbit/config.toml`; duels were 3-way codex/gemini/grok for experiments 1–3, then 4-way for experiment 4.

Standing at thread start:
- codex: 5/10
- claude: 7/9
- grok: 5/5
- gemini: 0/10

A pre-fix `orbit.duel.plan.add` sandbox bug caused some Gemini duels to fail mid-run earlier in the window. Incomplete artifacts do not score, so the 10 baseline losses above are all clean plan-vs-plan comparisons where Gemini's artifact landed.

## TL;DR

Across 5 Gemini-as-planner duels and a 4-model cross-read on identical input, **instruction surface (memory file or per-run prompt) reliably shifts what Gemini writes but does not shift which tools it calls.** Gemini ranked last on plan depth on every task shape we tested — implementation, audit, refactor, UX-taste — and lost every duel it participated in as planner. Gemini-as-arbiter performed adequately on the one pair it judged, recognizing and rewarding a bold design call its planner-self did not produce.

## Experiments

| # | Lever | File | Status |
|---|---|---|---|
| 1 | `GEMINI.md` *presence* (symlink to CLAUDE.md) | [01-presence-symlink](experiments/01-presence-symlink.md) | **Refuted** |
| 2 | `GEMINI.md` *content* (`## Planning` rubric appended) | [02-content-rubric](experiments/02-content-rubric.md) | **Refuted** |
| 3 | `PLANNING_DUEL_INSTRUCTION` strengthening (per-run prompt) | [03-prompt-strengthening](experiments/03-prompt-strengthening.md) | **Refuted** (implementation / refactor / audit on `gemini-2.5-pro`) |
| 4 | Task-shape rotated to UX / design-taste (cross-model on identical task) | [04-ux-taste-axis](experiments/04-ux-taste-axis.md) | **Inconclusive** (two-variable shift; Gemini still last on plan depth) |

## Findings

### A. Instruction surface shapes output, not tool selection

The strongest cross-experiment claim. Across the five Gemini-as-planner runs spanning experiments 01–04, every layer of plan production responded to the rubric *except* the one the rubric most directly addressed.

| layer | shifted by instruction surface? | evidence |
|---|---|---|
| internal deliberation (thoughts size) | yes | ORB-00149 thoughts ballooned 2.5× under the GEMINI.md `## Planning` rubric |
| output structure (sections, format, severities) | yes | verification sections, location-prefixed findings appeared in ORB-00151 / 00153 only after the rubric landed |
| output content quality (named identifiers, verification commands) | partial | verification commands appeared on ORB-00153 (zero on ORB-00146 baseline); named symbols on ORB-00153 were *fabricated* (`handleAuditHashChange`, `initAuditFilter`) rather than graph-discovered |
| tool selection (`graph.refs` / `graph.search`) | no | zero such calls across ORB-00146 / 00149 / 00151 / 00153, regardless of whether the rubric lived in memory (`GEMINI.md`) or in the per-run prompt (`PLANNING_DUEL_INSTRUCTION`) |

The arbiter on ORB-00153 made the cost of the bottom row explicit: Gemini fabricated symbol names that a single `graph.refs` call would have surfaced, and the fabrication lost the duel.

### B. Tool engagement and plan depth are separate levers

The first Gemini-as-planner session with non-zero `graph.refs` / `graph.search` calls was ORB-00155 (4 and 1 respectively). It was also the lightest plan in its 4-way comparison: 57 lines vs 84 / 94 / 233 for codex / grok / claude. The hidden-coupling section that opened the plan was 3 bullets vs Claude's 15+.

The rubric implicitly assumed engaging the right tools would pull plan depth along with it. It doesn't.

### C. Gemini ranked last on plan depth across every task shape tested

| run | task shape | outcome | depth in pair / pool |
|---|---|---|---|
| ORB-00146 | refactor | lost | bottom |
| ORB-00149 | module split | incomplete | bottom |
| ORB-00151 | site audit | lost | bottom |
| ORB-00153 | module split | lost | bottom |
| ORB-00155 | UX redesign | lost | bottom of 4-way |

Arbiter rationales (codex, claude, grok, gemini all sat as arbiter at different points) consistently cited the same Gemini-side weaknesses: graph-discovery gaps, hallucinated symbols, generic findings, factual inaccuracies, missing ADR content, "less specific about exact writeback path." The throughline is plan depth and code-grounded specificity, not a single failure mode.

### D. Gemini-as-arbiter is a different cognitive posture than Gemini-as-planner

On ORB-00154 Gemini-as-arbiter recognized and explicitly rewarded Claude's bold design call (metric-major layout + deliberate no-sort) over a more conventional alternative. On ORB-00155 Gemini-as-planner produced the most conventional plan in its pair (agent-major + colspan groups, structurally similar to Grok's). Reading completed artifacts is easier than producing them; the family that ranks bottom on planner depth can still arbite competently in the cases we observed.

### E. Model version was not a controlled variable

ORB-00146 / 00149 / 00155 ran on `gemini-3.1-pro-preview`; ORB-00151 / 00153 ran on `gemini-2.5-pro`. Cross-experiment claims therefore cross both task-shape and model-version boundaries. Experiment 03's "refuted on 2.5-pro for implementation" verdict is a clean within-model claim. Experiment 04's "graph tools engaged on UX-shaped task" reading sits at the intersection of task-shape and model-version changes; we didn't run the missing cell (3.1-pro-preview rubric run on an implementation-shaped task) to disambiguate.

### Subsidiary findings

- **Token counts in `summary.json` undercount Gemini severely.** Scoreboard reports 361 output tokens across 15 duels; a single session log shows 1,700+. The envelope parser at [cli_runner/envelope.rs](../../crates/orbit-engine/src/activity_job/cli_runner/envelope.rs) is not accounting for the `thoughts` field Gemini stores client-side. Dashboard signal is unreliable for Gemini; address separately.
- **`gemini` CLI workspace boundary is independent of orbit-exec sandbox.** Allowed dirs are `<workspace>` and `~/.gemini/tmp/<workspace>`. Anything else (e.g. `/tmp/plan.json`) fails mid-tool-call with "Path not in workspace". Pre-fix `orbit.duel.plan.add` failures correlate with this.
- **Without `-m`, `gemini -p` silently runs on `gemini-3.1-flash-lite`.** Orbit duels pin `-m pro` so they're safe, but ad-hoc invocations outside the pipeline run on a much weaker default.

## Caveats

This thread is **decision-grade for our local choices about which agents to put where**, not an objective ranking of model families.

- **Sample size.** N=5 Gemini-as-planner runs across the thread; N=1 UX-taste task; no within-model variance pass. Each model saw each task exactly once, so single-sample noise is indistinguishable from model behavior.
- **Single-codebase context.** Every task came from Orbit. Findings may not generalize to differently-structured projects.
- **Model version not held constant.** See finding E.
- **Arbiter family not held constant.** Codex, claude, gemini, and grok each arbited different runs. No within-arbiter consistency check.
- **Outcome = arbiter judgment, not ground truth.** A different arbiter could rank the same plans differently — and arbiter rationales themselves vary in what they weight (Gemini-as-arbiter weighted design boldness; Codex-as-arbiter weighted code-alignment and CSS hazard awareness).
- **Task selection.** Whatever was on deck in this window — no curated benchmark suite. Task mix is biased toward refactors, audits, JS module splits, and a single UX pass.
- **Infrastructure noise.** Pre-fix Gemini sandbox bug affected runs earlier in the window; ORB-00149 was incomplete (no winner.json). The tool-call envelope parser undercounts Gemini in scoreboard rollups (per-session `jq` counts are correct; scoreboard `tool_call_count: 0` is not).
- **Scope.** Findings apply to planning-duel plan quality, not to implementation, code-review, or other agent surfaces where the tool environment differs.

## Open questions

- Does the "instruction surface shapes output, not tool selection" pattern hold on `gemini-3.1-pro-preview` for implementation-shaped tasks? We tested the rubric on 2.5-pro (ORB-00151, ORB-00153) and saw graph engagement on 3.1-pro-preview for UX (ORB-00155), but never ran the post-rubric configuration on a refactor against 3.1-pro-preview directly. That single missing cell would tighten finding E.
- Is Gemini's "graph tools used on UX, not on refactor" a model-version effect (2.5 → 3.1), a task-shape effect, or both?
- Does Gemini-as-implementer (post-plan) exhibit the same graph-skipping pattern as Gemini-as-planner? Out of scope here; track in a separate observation when there's data.
- Would Gemini's plan depth shift with a *volume* rubric clause (e.g. "≥5 risks with mitigations, ≥100-line plan, ADR body inline")? We tested tool mandates, not volume mandates.
- Would arbiter family matter if every Gemini-vs-X pairing drew a non-Gemini arbiter? Codex-as-arbiter and Grok-as-arbiter both ruled against Gemini; the one Gemini-as-arbiter run saw a non-Gemini pair, so we don't have a Gemini-as-arbiter-on-Gemini-vs-X data point.

## Reproducing this

Cross-cutting commands. Per-experiment commands and per-experiment artifacts live in the experiment files; verbatim plans and the task description for experiment 04 live under [`references/`](references/).

```sh
# Per-run breakdown of duel outcomes for Gemini-family roles
jq '.runs[] | select(.roles | to_entries[] | .value.family == "gemini")' \
  .orbit/state/scoreboard/duel_plan.json

# Per-agent rollup. The retired CLI summary command has been replaced by the dashboard API.
curl -s http://127.0.0.1:7879/api/scoreboard >/dev/null
jq '.agents.gemini' .orbit/state/scoreboard/summary.json

# Tool-call audit for Gemini-family roles
orbit audit list --since 3d --limit 5000 --json \
  | jq '.[] | select(.role | test("gemini|^pro$|^flash"))'

# Model version per session (confirms which runs used 2.5-pro vs 3.1-pro-preview)
for f in ~/.gemini/tmp/orbit/chats/session-*.jsonl; do
  echo "=== $(basename $f) ==="
  jq -r 'select(.model != null) | .model' "$f" | sort -u
done
```
