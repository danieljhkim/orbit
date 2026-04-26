# v3 — Method & Caveats

This file documents how the `graph_v3` sweep was planned and conducted. The frozen report is [`RESULTS.md`](./RESULTS.md). Conventions governing this document's shape are in `benchmarks/CONVENTIONS.md`.

v3 was originally scoped as the final disposition round in the evidence series. Its results instead motivated the v4 diagnostic round and the accepted null-result narrative in `docs/design/knowledge-graph/5_null_result.md`.

## Harness git SHA at freeze

`a37f95cecca7d22711a4b47d9fddb7efed2a0f3b`

## Central intervention vs v2

A v2 review found that the `codex × hybrid 0/30 utilization` headline was not a behavioral measurement — it was a **tool-surface asymmetry**. Inspection of v2 run records shows every codex hybrid cell had `allowed_tools: ["exec_command"]` and the harness override `mcp_servers.orbit.enabled=false`. Codex was never offered `orbit_graph_*` as first-class MCP tools the way Claude was; it could only reach them by running `orbit tool run orbit.graph.*` as shell commands, competing against `rg` in codex's shell-tool heuristic.

v3's single experimental change is closing that access gap:

- **Codex gets MCP parity.** The `orbit-bench` MCP server is enabled for codex runs (inline `-c mcp_servers.orbit_bench.*` overrides), and codex's `enabled_tools` / `mcp_servers.*.enabled` toggles gate per-arm access the same way Claude's `--allowed-tools` / `--disallowed-tools` do.

Two non-experimental fixes land alongside — not design choices, just harness bugs v2 exposed:

- **Codex sandbox widened to `danger-full-access`.** v2 ran codex with `--sandbox read-only`, which caused 45 of 157 command_failures (`attempt to write a readonly database`) because the orbit binary's SQLite WAL needs write access. The intended v3 fix was `workspace-write` + `--add-dir <orbit_data_root>`, but smoke testing surfaced a second codex constraint: in non-interactive `codex exec` with `approval_policy="never"`, any sandbox short of `danger-full-access` causes MCP tool calls to be **cancelled** (`user cancelled MCP tool call`) rather than auto-approved. This was verified against the user's production `orbit` MCP server — the cancellation is codex-internal, independent of our server wiring. v3 therefore runs codex with `--sandbox danger-full-access`. Benchmark tasks never request writes (`edit`/`apply_patch` are not in `enabled_tools`) and `--ephemeral` keeps no session state between cells, so the widened sandbox materially only unblocks (a) SQLite WAL journaling and (b) MCP tool-call auto-approval. If a future codex release exposes a narrower MCP-auto-approve gate under `workspace-write`, v3.1 should adopt it.
- **Codex model pin.** v2 silently regressed from v1's `gpt-5.4` to `gpt-5.3-codex`. v3 pins the model explicitly via `DEFAULT_MODELS["codex"]` (env-overridable with `GRAPH_CODEX_MODEL`) and records the requested model in every run record so any future drift is visible. v3 holds codex at **`gpt-5.3-codex`** — matching v2 — so the v2→v3 delta isolates the access-parity intervention. Comparisons to v1's `gpt-5.4` codex numbers should account for the model difference.

Everything else is held constant vs v2:

- Same 10 fixtures (6 v1-carried + 4 v2-new) — no new fixture design.
- Same `orbit_graph_*` MCP tool surface (8 tools, descriptions unchanged from v2).
- Same arms (`no-graph`, `graph-only`, `hybrid`), same seeds-per-cell (3), same normalizer.
- Same indexer state (`.orbitignore` from v2 unchanged).

This keeps v3's signal single-variable: *does giving codex MCP access flip the utilization or cost curve?*

## Pre-registered disposition

Pre-registered so v3 would produce a decision, not another "inconclusive, design v4" outcome.

**The agent-facing `orbit_graph_*` MCP surface survives v3 only if BOTH:**

1. **Utilization:** Hybrid utilization ≥ 20% on at least one provider (i.e. at least one provider makes a graph-tool call in ≥ 6 of its 30 hybrid runs).
2. **Cost:** graph-only median (input + output) tokens ≤ 1.3× the matching no-graph median for the same (provider × fixture) cell.

**Otherwise the surface is culled** — MCP tools removed from the default server, `orbit.graph.*` stays as a CLI command only, and `5_null_result.md` is promoted from Draft to Accepted with v3's numbers as the closing entry.

Rationale: a schema-in-prompt overhead that agents don't use is pure cost; a tool surface that agents *do* use but can't solve within 1.3× the grep cost is a worse tool; either failure mode is disqualifying. Requiring both thresholds to pass together forces the tools to earn their seat on two independently-measured axes.

## Scope

- **Providers:** Claude (`sonnet-4-6`) + Codex (`gpt-5.3-codex`) — Claude's v2 gap must close. Subprocess spacing is added to stay under the subscription window.
- **Arms:** `no-graph`, `graph-only`, `hybrid`.
- **Fixtures:** 10 (same set as v2).
- **Seeds:** 3 per (provider × arm × fixture) cell.
- **Total runs:** 2 providers × 3 arms × 10 fixtures × 3 seeds = 180 cells.
- **Sweep IDs / seeds:** `20260423-194444-3b15a2` / `843516888` (Claude), `20260423-210616-9a7e7c` / `660936807` (Codex).
- **Sweep date:** 2026-04-23.

## Fixture list

Unchanged vs v2. See [`../v2/METHOD.md`](../v2/METHOD.md) §"Fixture list" for the v1-carried six, and [`../v2/V2_FIXTURES.md`](../v2/V2_FIXTURES.md) for the four v2-added fixtures.

## Known caveats (pre-run)

1. **`features.tool_call_mcp_elicitation` stays ENABLED for codex** (inverted from v2). Disabling it causes codex to auto-cancel MCP tool calls when a tool schema has any optional field. The trade-off is a small risk of mid-run elicitation loops; the worse trade-off is zero MCP calls actually running.
2. **MCP tool-name format differs per provider.** Claude exposes them as `mcp__orbit-bench__orbit_graph_<op>`; codex exposes them as `orbit.graph.<op>` (the server's advertised tool names). The normalizer's `_is_graph_tool_name` matches all observed formats so both providers count toward utilization.
3. **Tool-utilization column lands in `aggregate.py` before v3 runs.** ([T20260423-0524] was written pre-v2 but not wired. v3 is the first round where utilization is a first-class aggregate output, not a transcript rescan.)
4. **Codex cost is still reported as $0.** The Codex CLI does not emit billing; USD figures remain Claude-only.
5. **Claude pre-flight probe is lenient.** The probe (on haiku) succeeds if the CLI invocation returns exit=0 and either the graph tool was invoked OR `PROBE_OK` appears in the final message. Haiku sometimes shortcut-answers without calling the tool; that's fine — exit=0 + PROBE_OK is sufficient evidence the MCP config loaded. Main-run failures still fail loud.

## Reproduction

v3 uses the shared harness at `benchmarks/graph/scripts/` with `GRAPH_VERSION=v3`. To run a single cell:

```bash
GRAPH_VERSION=v3 python3 benchmarks/graph/scripts/run.py \
  --provider codex --arm hybrid --task locate-agentruntime --seed 1
```

To regenerate tables from run records:

```bash
make -C benchmarks graph-aggregate GRAPH_VERSION=v3
```

Or directly:

```bash
GRAPH_VERSION=v3 python3 benchmarks/graph/scripts/aggregate.py \
  --runs benchmarks/graph/v3/runs \
  --tasks benchmarks/graph/v3/tasks
```

## Task References

- **[T20260423-0506]** — v2 tool-surface trim (carried into v3 unchanged).
- **[T20260423-0524]** — `aggregate.py` utilization column (landing in v3).
- **[T20260423-0525]** — v2 tool-description rewrite (carried into v3 unchanged).
- **[T20260423-0607]** — pointer-only graph reads (deferred; not part of v3 — see below).

## Explicitly not in v3

- **Pointer-only graph reads ([T20260423-0607]).** Pre-registered for v4 *only if* v3 closes the access gap and utilization rises but cost fails the 1.3× threshold. If v3 fails the utilization threshold too, pointer-only is moot — redesigning the payload of tools agents don't call wastes engineering.
- **New fixtures.** v2 already stressed grep-hard cases; fixture-set instability would make a v2→v3 delta unreadable.
- **Tool description rewrites.** Held constant from v2 so the measured delta is attributable to access-path change alone.
