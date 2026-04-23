# Graph Benchmark Issues

This note records concrete token-usage issues observed in the preserved Codex benchmark transcripts for `locate-agentruntime`.

## Method

- These are rough token estimates, not provider-reported billing numbers.
- I estimated `rough_tokens ~= output_characters / 4`.
- The benchmark artifacts do not currently expose per-step Codex token accounting, so the best available proxy is the size of each command's captured output in the transcript.
- Large outputs that appear early are especially expensive because they are likely replayed into later cached context.

## Main Finding

The graph runs are spending budget on broad orientation dumps and duplicate verification, not on the direct answer path.

- For this task, the cheapest useful graph path is:

```bash
orbit tool run orbit.graph.search --input '{"query":"AgentRuntime","type":"symbol","kind":"trait","limit":10}'
orbit tool run orbit.graph.implementors --input '{"trait_selector":"symbol:crates/orbit-agent/src/runtime/runtime_trait.rs#AgentRuntime:trait"}'
orbit tool run orbit.graph.show --input '{"selector":"symbol:crates/orbit-agent/src/runtime/runtime_trait.rs#AgentRuntime:trait","depth":1,"siblings":false,"children":true}'
```

- The most expensive graph steps were broader than necessary for that workflow.

## Concrete Issues

| Issue | Example command | Transcript | Output chars | Rough tokens | Why it is expensive |
|---|---|---|---:|---:|---|
| Oversized graph overview | `orbit tool run orbit.graph.overview --input '{"prefix":"crates/orbit-agent/src"}'` | `runs/codex/hybrid/locate-agentruntime/2.transcript.json:12` | 65,756 | ~16,439 | Dumps `47` files and `427` symbols. This is far larger than needed after the trait search already succeeded. |
| Full skill file loaded into run context | `sed -n '1,220p' .orbit/resources/skills/orbit-graph/SKILL.md` | `runs/codex/graph-only/locate-agentruntime/1.transcript.json:5` | 5,563 | ~1,391 | Loads instructions into the conversation before any task-specific graph call. This is fixed overhead for graph-mode runs. |
| Noisy refs output around the trait | `orbit tool run orbit.graph.refs --input '{"selector":"symbol:crates/orbit-agent/src/runtime/runtime_trait.rs#AgentRuntime:trait","limit":50}'` | `runs/codex/graph-only/locate-agentruntime/1.transcript.json:21` | 6,183 | ~1,546 | Returns doc sections and README hits in addition to runtime implementors, so the model pays for irrelevant references. |
| Broad pack of all impl blocks | `orbit tool run orbit.graph.pack --input '{"selectors":["symbol:crates/orbit-agent/src/providers/claude/claude_runtime.rs#ClaudeRuntime:impl","symbol:crates/orbit-agent/src/providers/codex/codex_runtime.rs#CodexRuntime:impl","symbol:crates/orbit-agent/src/providers/gemini/gemini_runtime.rs#GeminiRuntime:impl","symbol:crates/orbit-agent/src/providers/mock_agent/mock_agent_runtime.rs#MockAgentRuntime:impl","symbol:crates/orbit-agent/src/providers/ollama/ollama_runtime.rs#OllamaRuntime:impl"]}'` | `runs/codex/graph-only/locate-agentruntime/1.transcript.json:24` | 4,509 | ~1,127 | Helpful, but still a sizable blob of source that the model later restates almost directly. |
| Broad search that pulls in benchmark YAML noise | `orbit tool run orbit.graph.search --input '{"query":"AgentRuntime","limit":10}'` | `runs/codex/graph-only/locate-agentruntime/1.transcript.json:9` | 1,975 | ~494 | Returns `benchmarks/graph/tasks/locate-agentruntime.yaml` config keys before code symbols. |
| Duplicate raw file verification after graph already answered the question | Multiple `sed -n` and `nl -ba ... | sed -n` reads over runtime files | `runs/codex/hybrid/locate-agentruntime/2.transcript.json:19-42` | 31,272 total | ~7,818 total | The graph had already identified the trait and all five implementors, but the run still reread full provider files and then reread them with line numbers. |
| Broad no-graph baseline search is also noisy | `rg -n "AgentRuntime" crates .` | `runs/codex/no-graph/locate-agentruntime/1.transcript.json:7` | 12,909 | ~3,227 | Includes `AGENTS.md`, `CLAUDE.md`, benchmark YAML, and design docs. This is wasteful too, but it is still smaller than the giant graph overview dump. |

## Transcript Notes

### Hybrid rerun: the direct answer path was already available early

These two commands were small and sufficient:

```bash
orbit tool run orbit.graph.search --input '{"query":"AgentRuntime","type":"symbol","kind":"trait","limit":10}'
orbit tool run orbit.graph.implementors --input '{"trait_selector":"symbol:crates/orbit-agent/src/runtime/runtime_trait.rs#AgentRuntime:trait"}'
```

- The search result at `runs/codex/hybrid/locate-agentruntime/2.transcript.json:11` already identifies `AgentRuntime` in `crates/orbit-agent/src/runtime/runtime_trait.rs`.
- The implementor query at `runs/codex/hybrid/locate-agentruntime/2.transcript.json:15` returns all five runtime implementors directly.
- The very large overview at `runs/codex/hybrid/locate-agentruntime/2.transcript.json:12` came between those steps and appears unnecessary for this task.

### Graph-only: the expensive parts were mostly broad graph context

- `runs/codex/graph-only/locate-agentruntime/1.transcript.json:9` uses an unfocused search that surfaces benchmark task YAML.
- `runs/codex/graph-only/locate-agentruntime/1.transcript.json:21` uses `orbit.graph.refs`, which returns many non-implementor references.
- `runs/codex/graph-only/locate-agentruntime/1.transcript.json:24` uses `orbit.graph.pack` to pull the full impl bodies for all five runtimes.

Together, those three graph-tool outputs account for about `12,667` characters, or roughly `3,167` tokens, before counting the skill read overhead.

## Rough Per-Run Breakdown

These totals sum captured command output size by category.

| Run | Dominant category | Captured chars | Rough tokens |
|---|---|---:|---:|
| `graph-only/1` | Graph tool output | 16,914 | ~4,229 |
| `graph-only/1` | Skill file read | 5,563 | ~1,391 |
| `no-graph/1` | Source reads | 20,362 | ~5,091 |
| `no-graph/1` | Ripgrep output | 13,590 | ~3,398 |
| `hybrid/2` | Graph tool output | 67,917 | ~16,979 |
| `hybrid/2` | Source reads | 31,272 | ~7,818 |

## Recommendations

- For narrow symbol-location tasks, do not call `orbit.graph.overview` after `orbit.graph.search` already found the target symbol.
- Prefer `search -> implementors -> show` over `overview -> refs -> pack`.
- Avoid broad `orbit.graph.search` calls without `type`, `kind`, or `prefix` filters.
- Tighten `orbit.graph.refs` usage or post-filter its results so docs and benchmark YAML do not dominate the output.
- If graph tools already provide the trait and implementor list, do not reread every provider source file unless the benchmark explicitly requires code-level behavioral summaries.
