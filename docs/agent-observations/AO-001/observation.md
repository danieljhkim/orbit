---
id: AO-001
title: Tool surface preference splits by model family
owner: claude
last_updated: 2026-05-18
recorded: 2026-05-18
tags: [tool-surface, mcp, cli, model-family, codex, learnings]
---

# Tool surface preference splits by model family

*Editorial note: References below to the unified search surface with `kind=learning` reflect a mechanical rewrite. The original observation referred to the phase-1 per-domain tool (retired by [ORB-00202] in favor of unified `orbit.search` with `kind: "learning"`). Body text otherwise preserves the original analysis.*

**Window:** 2026-05-15 → 2026-05-18 (3 days)
**Source:** `orbit audit list --since 30d` — db was reset ~3 days prior, so the 30d query returned 3d
**Role normalization:** raw audit rows contain many self-identifications (`gpt-5`, `gpt-5-codex`, `gpt-5.5`, `grok-build`, `grok-4`, `claude-opus-4-7`, `pro`, etc.). These were consolidated into the four canonical agent families — **codex, claude, grok, gemini** — plus **agent** (orchestrator) and **admin** (human/admin CLI). The inconsistent self-identification is itself a finding; see §Self-identification below.

## TL;DR

For identical operations on the same workspace with identical wiring, **codex picks CLI ~92% of the time; every other agent (claude, grok, gemini) picks MCP ~70%+ of the time.** Not a coverage gap, not a config issue — a training-prior difference.

## Surface split by role

Tool-call events only (`subcommand == "run"` → CLI, `subcommand == "run-mcp"` → MCP). Excludes admin/job/learning/audit events that aren't routed through `tool run*`.

| role | tool_calls | mcp | cli | mcp% |
|---|---:|---:|---:|---:|
| codex | 1266 | 105 | 1161 | **8** |
| agent (orchestrator) | 822 | 749 | 73 | **91** |
| grok | 819 | 610 | 209 | 74 |
| admin | 655 | 0 | 655 | 0 |
| claude | 466 | 322 | 144 | 69 |
| gemini | 138 | 98 | 40 | 71 |

`agent` (91% MCP) is mostly claude under a generic role tag — events emitted through code paths that don't pass through a model identifier (notably the preToolUse hook firing `orbit.search` (kind=learning), which accounts for ~530 of the 749 MCP calls in that row). If the `agent` events were merged into claude, claude's MCP% would jump from 69% to ~83% and the sample would roughly double; the codex/claude gap widens accordingly. Left unmerged in the data to keep the role boundary visible. `admin` is human-invoked CLI by definition, so 0% MCP is expected.

The three named model rows (claude, grok, gemini) cluster tightly at 69–74% MCP. Codex sits at 8%. The gap is the finding.

## It is not a coverage gap

For tools with full CLI/MCP parity, codex still picks CLI for the same operations the other three pick MCP for. Cells are `mcp/cli`:

| tool | codex | claude | grok | gemini |
|---|---|---|---|---|
| `orbit.graph.show` | 0 / 273 | 34 / 0 | 84 / 53 | 12 / 4 |
| `orbit.graph.search` | 0 / 179 | 34 / 0 | 112 / 85 | 0 / 2 |
| `orbit.task.show` | 18 / 197 | 31 / 6 | 31 / 17 | 14 / 5 |
| `orbit.task.update` | 43 / 123 | 34 / 9 | 43 / 2 | 2 / 2 |
| `orbit.graph.pack` | 0 / 88 | 12 / 0 | 28 / 6 | 18 / 1 |
| `orbit.search` | 13 / 52 | 4 / 0 | 6 / 0 | 2 / 0 |

Codex's MCP usage of `orbit.task.update` (43 MCP, 123 CLI) is the key counter-evidence to "codex never uses MCP." It proves codex *can* and *does* reach for MCP — it just defaults to CLI even when MCP is wired and identical.

## Knock-on effect: `orbit.search` (kind=learning)

| role | mcp | cli |
|---|---:|---:|
| grok | 254 | 0 |
| gemini | 48 | 0 |
| claude | 42 | 0 |
| codex | **0** | **0** |

Codex never calls `orbit.search` (kind=learning) — not via CLI either. The cause is *not* a coverage gap. Learnings get surfaced to agents via two reminder paths:

1. Claude's `preToolUse` hook
2. The MCP server's learning sidecar

Codex uses CLI exclusively and runs without the preToolUse hook, so it never gets reminded that learnings exist. The result is that agents who reach for shell are also the agents flying blind on project conventions encoded in Orbit learnings.

**This is the most actionable finding in this window.** Tool-surface preference isn't just aesthetic — it gates access to the surrounding context system.

## Self-identification

A secondary finding from the data-prep step: agents identify themselves inconsistently in audit events. Within this 3-day window the same OpenAI lineage appeared under four different `role` values (`gpt-5`, `gpt-5-codex`, `gpt-5.5`, `codex`); the same xAI lineage under four (`grok`, `grok-4`, `grok-4.3`, `grok-build`); the same Anthropic lineage under three (`claude`, `claude-opus-4-7`, `claude-opus-4-7-build`); Gemini under three (`gemini`, `gemini-3.1-pro`, `pro`).

Separately, the generic `agent` role tag (822 events in this window) is mostly claude — it appears on events emitted through code paths that don't pass through a model identifier. This is a third class of mis-attribution beyond variant-name aliasing and self-id typos: events that are correctly attributed to *something*, but the something is generic enough to obscure the model behind it.

If the audit log is going to be a useful longitudinal signal, the canonical-name discipline needs to be enforced at write time — either by the MCP server / CLI when emitting the audit event, or by normalizing on read. The current state forces every analysis to reinvent the alias map and to guess at what `agent` means.

## Other notable rows

- **claude's `orbit.adr.list`**: 0 MCP, 47 CLI — outlier against claude's own 69% MCP baseline. Probably a single scripted session; flagging in case it isn't.

## Caveats

- **3 days of data, not 30.** The local audit db was reset on 2026-05-15. Treat absolute counts as suggestive, not stable. The per-tool ratios are the load-bearing signal.
- **Volume reflects assignment, not disposition.** Different agents had different workloads in this window. Cross-model comparisons should be on *ratios*, not totals.
- **Role attribution depends on self-identification** — see §Self-identification. Pre-consolidation, the same model could appear under multiple role labels; post-consolidation, the canonical names group them but can't fix events tagged with a misleading role in the first place.

## Open questions

- Does codex's CLI bias survive when MCP is the only surface wired? (Soft preference vs. hard prior.)
- Would surfacing learnings inside CLI tool responses (e.g. as a footer on `orbit tool run`) close the codex blind spot, or do the OpenAI models discard reminders too?
- How does the split look over a 30d window once the audit db has accumulated one?
- What would it take to enforce canonical role names at audit-write time?

## Reproducing this

```bash
orbit audit list --since 30d --limit 100000 --json > /tmp/audit.json

# Role × surface, with alias consolidation
jq -r '
  def normalize(r):
    if r == "gpt-5" or r == "gpt-5-codex" or r == "gpt-5.5" or r == "codex" then "codex"
    elif r == "claude-opus-4-7" or r == "claude-opus-4-7-build" or r == "claude" then "claude"
    elif r == "grok" or r == "grok-4" or r == "grok-4.3" or r == "grok-build" then "grok"
    elif r == "gemini" or r == "gemini-3.1-pro" or r == "pro" then "gemini"
    else r
    end;
  [.[] | select(.subcommand == "run-mcp" or .subcommand == "run")]
  | map(. + {role: normalize(.role)})
  | group_by(.role)
  | map({role: .[0].role,
         mcp: [.[] | select(.subcommand == "run-mcp")] | length,
         cli: [.[] | select(.subcommand == "run")]     | length})
  | map(. + {pct: ((.mcp * 100 / (.mcp + .cli)) | floor)})
  | sort_by(-(.mcp + .cli)) | .[]
  | "\(.role)\t\(.mcp)\t\(.cli)\t\(.pct)%"
' /tmp/audit.json
```
