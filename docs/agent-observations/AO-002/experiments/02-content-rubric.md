---
parent: ../observation.md
recorded: 2026-05-18
lever: GEMINI.md content (Planning rubric)
status: refuted
artifact: ORB-00149 planner_a (duel did not complete)
---

# Experiment 02 — GEMINI.md content rubric

## Setup

Converted `GEMINI.md` from a symlink to a real file at 2026-05-17 21:24 PDT, appending a `## Planning` rubric that explicitly required: `orbit.graph.refs` per moved symbol, `orbit.graph.search` for consumer enumeration, no hedge language, exact verification commands (`rg` / `cargo` / `make ci-fast` / `curl`), and hidden-coupling enumeration in step 1.

What was added:

```
## Planning

When planning a refactor, module split, or any task that moves code:
- Call orbit.graph.refs on every symbol you propose to move, BEFORE drafting
  the plan. Enumerate consumers in the plan body by name.
- Call orbit.graph.search to find call sites of moved types.
- Name specific imports, functions, and modules in the plan — never
  "this should work" or "verify it compiles".
- Specify exact verification commands: `rg`, `cargo`, `make ci-fast`, curl.
- If you find hidden coupling not reflected in the task description,
  enumerate it in step 1 of the plan.
```

Ran one Gemini-as-planner duel against the new content (jrun planner_a artifact for ORB-00149, written at 22:05 PDT — 41 min after the content was in place). Duel did not complete (only `planner_a.md` on disk; no `winner.json`; no scoreboard entry). Plan-quality verdict is independent of the missing arbiter.

## Result

**Refuted.** All 5 rubric bullets ignored. The rubric is in context (verified per experiment 1's loading check) and ignored.

Rubric compliance on the resulting plan:

| GEMINI.md `## Planning` requirement | observed | status |
|---|---|---|
| `orbit.graph.refs` per moved symbol, before drafting | 0 `graph.refs` calls | ❌ |
| `orbit.graph.search` per module boundary | 0 `graph.search` calls | ❌ |
| Name symbols / functions / modules by identifier | Names ~10 symbols, all lifted verbatim from the task description's scope list | partial (no graph-derived discovery) |
| Exact verification commands in the plan body | 0 commands of any kind | ❌ |
| Enumerate hidden coupling in step 1 | None enumerated; one generic ES-circular-import note in Risks | ❌ |

Tool-call profile, identical in shape to experiment 1:

```
ORB-00146 (no rubric):    graph.pack ×1, read_file ×4
ORB-00149 (with rubric):  graph.pack ×1, read_file ×1, write_file ×3, run_shell_command ×2
```

One `graph.pack` then fall-through to fs/builtin.

One thing did change: Gemini's `thoughts` ballooned from ~14,000 chars on ORB-00146 to ~36,938 chars on ORB-00149, with output tokens up from 1,739 to 5,988. **2.5× more reasoning, plan slightly shorter** (34 lines / ~370 words vs 35 lines / ~600 words). More cogitation, equally generic output. The rubric appears to expand Gemini's internal deliberation without redirecting its tool selection.

This refutes the assumption that *content* (rather than mere presence) of `GEMINI.md` shifts behavior. Combined with experiment 1, both instruction-file levers are now refuted. Moves the hypothesis to per-run prompt steering — see [03-prompt-strengthening](03-prompt-strengthening.md).

## Reproducing

```sh
# Inspect the post-content-rubric Gemini plan (no winner — incomplete duel)
cat ~/.orbit/tasks/workspaces/orbit-*/ORB-00149/artifacts/files/planning-duel/planner_a.md

# Compare reasoning trace sizes
ls -lt ~/.gemini/tmp/orbit/chats/ | head
```
