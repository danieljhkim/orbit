---
parent: ../observation.md
recorded: 2026-05-18
lever: GEMINI.md presence (symlink to CLAUDE.md)
status: refuted
artifact: ORB-00146 (jrun-20260518-0353)
---

# Experiment 01 — GEMINI.md presence

## Setup

Created `GEMINI.md` as a symlink to `CLAUDE.md` at 2026-05-17 20:09 PDT. Ran one Gemini-as-planner duel (jrun-20260518-0353, ORB-00146) at 20:53 PDT — 44 min after the symlink was in place.

## Result

**Refuted.** GEMINI.md is in the context window. It is not influencing reasoning.

- Outcome: still a loss. Arbiter cited Codex's graph-based discovery of CLI-internal coupling as the winning factor.
- Verified GEMINI.md was loaded: `gemini -p "list files in your context"` returns `/users/daniel/workspace/repos/orbit/gemini.md` in the loaded-context list, with tier label "Project Instructions".
- Grepped Gemini's full reasoning trace from the JSONL session log (13,988 chars of thoughts + 390 chars of content across 16 turns) for 12 CLAUDE.md-distinctive phrases (`agent-main`, `ci-fast`, `ARCHITECTURE.md`, `pub(crate)`, `workspace.lints`, `thiserror`, `denyModify`, etc.) — **0 of 12 appear anywhere**.

CLAUDE.md is an operational doc (commit / branch / lint rules); it is not a planning rubric, so loading it does not teach Gemini *how* to plan. This motivated experiment 2: append a planning-specific rubric to `GEMINI.md` and see whether content (rather than mere presence) shifts behavior.

### Head-to-head plan comparison

| | Gemini planner_a | Codex planner_b |
|---|---:|---:|
| lines | 35 | 30 |
| words | ~600 | ~700 |
| specific identifiers named in plan body | 0 | 7 |
| verification commands (rg / cargo / make / curl) | 0 | 6 |
| risk items | 3 (generic) | 4 (each names a specific failure mode) |
| hedge phrases ("this should work", "should compile") | 2 | 0 |

Codex named `parse_since`, `audit_event_to_json`, `job_catalog_to_json_with_last_run`, `job_run_to_json`, `task_to_json_with_sidecars`, `task_locks_json`, `learning_to_json` — the coupling Codex discovered via graph tools. Gemini named zero functions or symbols.

## Reproducing

```sh
# Read the head-to-head plan artifacts the arbiter saw
ls ~/.orbit/tasks/workspaces/orbit-*/ORB-00146/artifacts/files/planning-duel/

# Verify GEMINI.md is in Gemini's context
gemini -p "list files currently in your context" -o json --approval-mode yolo -m pro
```
