# Codex Work Assignment — v4 Graph Benchmark

This file specifies the v4 work delegated to codex. The Claude side handles
synthetic-fixture authoring and harness extensions; codex handles
production-fixture YAMLs and the v3 results writeup. See
`benchmarks/graph/v4/METHOD.md` for the round's design and rationale.

The split is intentional: codex's session-long track record of catching
fictional code references (e.g. the `LoopAuditSink` / `AgentRuntime::run_session`
/ `Policy::resolve` targets that didn't exist) makes codex the better author
of fixtures that depend on real-code ground-truth enumeration. The synthetic
side stays with Claude because that work needs tight feedback loops with the
indexer, which Claude has already validated end-to-end.

---

## Hard dependency: structured-oracle YAML schema

**Do not start Task A (production YAMLs) until this section's status is "LOCKED".**

Status: **LOCKED — matches the committed v4 synthetic fixture YAMLs and structured oracle**

The locked YAML schema for v4 fixtures is below. The fixture YAMLs you author
go under `benchmarks/graph/v4/tasks/<id>.yaml` and must conform.

```yaml
# v4 fixture YAML

task_id: <fixture-id>                     # filename minus .yaml; matches METHOD.md inventory
class: <graph-strength|precision-gap|payload-volume|selector-ambiguity>
difficulty: <easy|medium|hard>

mode: <production|synthetic>
synthetic_code_path: <relative-path>      # only when mode: synthetic; e.g. "_fixture_code/foo.rs"
hybrid_eligible: <true|false>             # whether this fixture runs in the hybrid arm

prompt: |
  <natural-language prompt for the agent>
  <must explicitly scope to "Orbit production code in <crate>" for production fixtures,
   or "in benchmarks/graph/v4/_fixture_code/<file>" for synthetic fixtures>
  <must end with an explicit JSON shape:
   {"answer": ["<item>", ...], "excluded": ["<item>", ...]}>

oracle:
  structured:                             # v4 default. Falls back to grep/cmd/judge only
    item_kind: <file_path|file_line|symbol_name|symbol_with_value|crate_name>
    scoring: <exact_set|superset_ok|subset_ok>
    case_sensitive: false
    ground_truth:
      answer:
        - <item>                          # ordered list, but matched as a set
        - <item>
      excluded_acceptable:                # optional; if agent puts these in "excluded",
        - <item>                          # that's allowed but not required
                                          # primary grading is on "answer" only
    deny_list:                            # patterns the agent's "answer" MUST NOT contain;
      - pattern: <substring>              # substring match only; not regex
        reason: <one-line explanation>

notes:                                    # optional; freeform authoring notes
  - <note>
```

**Item-kind shapes:**

- `file_path` — relative path from repo root, forward slashes. Example:
  `crates/orbit-knowledge/src/service/history.rs`
- `file_line` — `file_path:line` from repo root. Example:
  `crates/orbit-knowledge/src/service/history.rs:337`
- `symbol_name` — fully qualified symbol. Example: `RefName::new`
- `symbol_with_value` — `<name>=<value>` for const fixtures. Example:
  `DEFAULT_POLICY_NAME="default"`
- `crate_name` — just the crate name. Example: `orbit-knowledge`

**Scoring modes:**

- `exact_set` — answer must equal ground-truth set; both extras and misses fail
- `superset_ok` — answer must contain ground truth; extras are allowed
- `subset_ok` — answer must be subset of ground truth; misses are allowed (rare;
  used when ground truth has plausible disagreement)

The structured oracle only grades the `answer` list as strings. If a fixture
needs body-comparison payload, encode it as exact `symbol_with_value` labels
or keep the graded `answer` as symbols and put any explanatory prose in
`notes`; do not rely on the oracle to parse free-form summaries.

---

## Task A: 7 production fixture YAMLs

Output: one YAML per fixture under `benchmarks/graph/v4/tasks/<id>.yaml`,
conforming to the schema above.

### Per-fixture specifications

All targets are locked in `METHOD.md` §"Fixture inventory". The acceptance
criteria below are intentionally specific so you don't have to re-derive
ground truth from the design doc.

#### A.1 `function-as-value-vs-direct-call.yaml`

- **target:** `RefName::new` in `orbit-knowledge`
- **prompt direction:** "Find sites in Orbit production code where `RefName::new`
  is passed as a value (e.g. `.map(RefName::new)`), excluding sites that call
  `RefName::new(...)` directly. Output JSON: `{answer, excluded}`."
- **item_kind:** `file_line`
- **ground-truth enumeration command:**
  ```
  rg -n "\.map\(RefName::new\)" crates/*/src
  ```
  Expected hits: 3 sites (verified):
  - `crates/orbit-knowledge/src/graph/object_store.rs:161`
  - `crates/orbit-cli/src/command/task/history.rs:240`
  - `crates/orbit-cli/src/command/graph.rs:256`
  Explicit non-answer: `crates/orbit-knowledge/src/service/history.rs:337`
  is `.and_then(|branch| RefName::new(branch).ok())`, a direct call inside a
  closure, not a function item passed as a value.
- **deny_list:** patterns that look like as-value but are actually direct calls,
  e.g. `RefName::new(arg)` where `arg` is a variable name; populate from
  `rg -n "RefName::new\(" crates/*/src` minus the as-value sites.
- **hybrid_eligible:** true
- **acceptance:** running `orbit graph refs --selector ...RefName::new...` should
  return a SUPERSET of the answer (graph returns both as-value and direct-call
  sites without distinction; the test is whether the agent filters correctly).

#### A.2 `impl-divergence-trait-method.yaml`

- **target:** `AuditSink::emit` (4 production impls)
- **prompt direction:** "List all production impls of `AuditSink::emit` and
  classify how each impl handles the `LoopAuditEvent::PolicyDenial` variant.
  Output JSON: `{answer: [\"<TypeName>::emit=<canonical-label>\", ...], excluded: []}`."
- **item_kind:** `symbol_with_value`
- **ground truth (verified):**
  - `NullSink::emit=no-op`
  - `InMemorySink::emit=stores_all_events`
  - `JsonlFileSink::emit=writes_all_events_to_jsonl`
  - `EnforcedAuditSink::emit=mirrors_policy_denial_and_blocks_disallowed_tool_calls`
- **scoring:** `exact_set`
- **hybrid_eligible:** false (graph-only diagnostic)

#### A.3 `references-vs-callers-tool-registry-register.yaml`

- **target:** `ToolRegistry::register` in `orbit-tools`
- **prompt direction:** "Find production source files that contain call-sites
  invoking the `orbit_tools::ToolRegistry::register(...)` method. Exclude
  unrelated `register(...)` methods on other registry types, and exclude
  declaration-only files. Output JSON: `{answer, excluded}`."
- **item_kind:** `file_path`
- **ground-truth enumeration:**
  ```
  rg -n "(registry|self)\.register\(" crates/orbit-tools/src crates/orbit-core/src/runtime/builder.rs
  ```
  Answer set is the distinct file paths from that command:
  `crates/orbit-tools/src/builtin/{fs,git,github,net,orbit,proc,time}/mod.rs`
  plus `crates/orbit-core/src/runtime/builder.rs`.
- **deny_list:** unrelated registry files:
  `crates/orbit-agent/src/runtime/backend.rs`, `crates/orbit-engine/src/executor/registry.rs`,
  and the method definition file `crates/orbit-tools/src/registry.rs`.
- **hybrid_eligible:** false (selector-ambiguity diagnostic)
- **note:** This is a selector-ambiguity probe. The interesting variable is
  whether the agent uses `orbit.graph.refs` vs `orbit.graph.callers` vs
  `orbit.graph.search` for this question, and whether the choice produces
  different result sets. Make the prompt natural-sounding (don't suggest a
  tool); the ambiguity is the whole point.

#### A.4 `deps-downstream-orbit-knowledge.yaml`

- **target:** crates that transitively depend on `orbit-knowledge`
- **prompt direction:** "List all crates in this workspace that depend on
  `orbit-knowledge` either directly or transitively (depth ≤ 2). Output JSON:
  `{answer, excluded}`."
- **item_kind:** `crate_name`
- **ground truth (verified via `rg "orbit-knowledge|orbit-tools|orbit-cli" crates/*/Cargo.toml -l`):**
  - Direct: `orbit-tools`, `orbit-cli`
  - Transitive (depth 2 via orbit-tools): `orbit-core`, `orbit-engine`, `orbit-agent`
  - Total: 5 crates
- **deny_list:** crates that don't depend on orbit-knowledge — `orbit-common`,
  `orbit-policy`, `orbit-exec`, `orbit-store`, `orbit-mcp`. Verify each.
- **hybrid_eligible:** true
- **note:** validate the depth-2 set before locking; cargo's transitive deps
  may include surprises. `cargo tree -i orbit-knowledge --workspace --depth 2` is the
  authoritative source.

#### A.5 `reverse-export-orbit-error.yaml`

- **target:** every module that re-exports `OrbitError`
- **prompt direction:** "List every Rust module (file path) in Orbit production
  code that re-exports `OrbitError` via `pub use ...OrbitError`. Output JSON:
  `{answer, excluded}`."
- **item_kind:** `file_path`
- **ground truth (verified):**
  - `crates/orbit-common/src/types/mod.rs` (re-exports from `error::`)
  - `crates/orbit-core/src/lib.rs` (re-exports from `orbit_common::types::`)
- **deny_list:** the original definition site — `crates/orbit-common/src/types/error.rs`
  (defines `pub enum OrbitError`, not a re-export). Also the variant constructors
  (`OrbitError::InvalidInput`, etc.) — those aren't re-exports of the type.
- **hybrid_eligible:** true

#### A.6 `module-surface-orbit-mcp.yaml`

- **target:** public surface of crate `orbit-mcp`
- **prompt direction:** "List all `pub` items in the public root of crate
  `orbit-mcp` (i.e. items reachable as `orbit_mcp::<name>`). Output JSON:
  `{answer, excluded}`."
- **item_kind:** `symbol_name`
- **ground truth (verified via `rg "^pub" crates/orbit-mcp/src/lib.rs`):**
  - `OrbitToolServer` (re-exported from `adapter`)
  - `McpHost` (trait)
  - `serve_stdio` (async fn)
- **deny_list:** items that are `pub` inside submodules but NOT re-exported at
  the crate root. Run `rg "^pub" crates/orbit-mcp/src/` and exclude
  symbols not in `lib.rs`.
- **hybrid_eligible:** false (selector-ambiguity diagnostic — agent must choose
  between `overview`, `search`, and `show` calls)

#### A.7 `const-value-extraction.yaml`

- **target:** all `pub const` declarations (not `pub const fn`) in
  `orbit-common/src/types/`
- **prompt direction:** "List all `pub const` declarations (data, not functions)
  in `orbit-common/src/types/` along with their declared values. Output JSON:
  `{answer: [\"<NAME>=<value>\", ...], excluded: []}`."
- **item_kind:** `symbol_with_value`
- **ground truth (verified via `rg "^pub const \w+" crates/orbit-common/src/types/`):**
  - `AUDIT_ENVELOPE_SCHEMA_VERSION=1`
  - `ACTIVITY_REF_PREFIX="activity:"`
  - `V2_TOOL_WILDCARD_ROOTS=["orbit.graph.","orbit.task.","orbit.audit.","fs.","proc."]`
  - `DEFAULT_POLICY_NAME="default"`
  - `UNRESTRICTED_FS_PROFILE="unrestricted"`
  - `EXECUTOR_RESOURCE_SCHEMA_VERSION=2`
  - `POLICY_RESOURCE_SCHEMA_VERSION=2`
- **deny_list:** `pub const fn` items (these are functions, not data) — e.g.
  `default_job_max_active_runs()`, `default_max_iterations()`,
  `default_retry_backoff_seconds()`, `all_agent_families()`. Also any
  `pub const` outside `src/types/` (the prompt scopes to that subdir).
- **hybrid_eligible:** false (payload-volume diagnostic)

### Review back to Claude

When the YAMLs are drafted, ping back. Claude will:
1. Run each fixture's ground-truth enumeration command and verify the
   `answer` set matches.
2. Run `orbit tool run orbit.graph.<op>` against each target and confirm the
   graph result is a superset/subset/exact-match per the fixture's design.
3. Cross-check `deny_list` against the actual graph/grep output for false
   positives.

Push back on anything in this spec that doesn't match what you find in the
codebase. The session has multiple precedents for me being wrong about
production-code shape.

---

## Task B: `v3/RESULTS.md`

Output: `benchmarks/graph/v3/RESULTS.md`. The file was supposed to be written
during the v3 sweep but never was.

### What you have to work with

- **Run records:** `benchmarks/graph/v3/runs/{claude,codex}/{no-graph,graph-only,hybrid}/<task>/<seed>.json`
  (180 records, all parseable JSON)
- **Sweep metadata:** `benchmarks/graph/v3/runs/_sweeps/{claude,codex}/<sweep_id>/{order.json,results.json}`
- **Aggregator:** `benchmarks/graph/scripts/aggregate.py` — invoke as:
  ```
  GRAPH_VERSION=v3 python3 benchmarks/graph/scripts/aggregate.py \
    --runs benchmarks/graph/v3/runs --tasks benchmarks/graph/v3/tasks
  ```
- **Per-cell cost ratios already computed:** see `docs/design/knowledge-graph/5_null_result.md`
  §"Disposition" — the 10-row per-fixture cost table is the source of truth for
  the per-cell threshold reading.
- **Templates:** `benchmarks/graph/v1/RESULTS.md` and `benchmarks/graph/v2/RESULTS.md`
  show the established RESULTS.md shape.
- **Closing reference:** `docs/design/knowledge-graph/5_null_result.md` is the
  evidence-log entry that consumes the v3 results. RESULTS.md should be
  consistent with the disposition section there (per-cell, not aggregate).

### Required sections (per `benchmarks/CONVENTIONS.md` shape)

1. **Round metadata** — sweep date, harness git SHA, total cells, providers,
   arms, fixtures.
2. **Headline numbers** — codex hybrid 0/30 → 23/30 v2→v3 utilization flip;
   claude hybrid 0/30; per-arm pass rates.
3. **Primary table** — provider × arm × task_class. Output the aggregator's
   primary table verbatim.
4. **Cost analysis** — per-cell cost ratios (the 10-row table from
   `5_null_result.md` §"Disposition"). Be explicit that this is per-cell, not
   aggregate.
5. **Tool utilization audit** — codex hybrid graph-call rate by fixture; the
   `impact-scope-strategy-callsites` 0/3 outlier and the firehose
   `impact-tool-context-struct-literals` (12.43× cost on codex graph-only).
6. **Disposition** — quote the pre-registered threshold from `v3/METHOD.md`,
   then state the outcome. Match the framing in `5_null_result.md`
   §"Disposition" (retain on utilization; cost is mixed per-cell).
7. **Errors** — the 3 errored runs from the aggregator output.
8. **Caveats carried into v4** — point at `v3/METHOD.md` caveats and
   `5_null_result.md` §"Methodological postscript" (per-cell vs aggregate
   pre-registration lesson).

### Acceptance

- `RESULTS.md` exists at `benchmarks/graph/v3/RESULTS.md`.
- Numbers in the file reproduce from `aggregate.py` output and from the
  per-cell table in `5_null_result.md`. The only permitted additional
  derivation is the codex hybrid graph-call-rate-by-fixture table, computed
  directly from frozen run records' `tool_calls` fields.
- Cost section reports per-cell, with aggregate explicitly secondary.
- Disposition is consistent with `5_null_result.md` (retain; mixed per-cell;
  utilization-carried). No drift from the closing entry.
- Errors table includes the 3 errored runs (claude no-graph
  locate-loopaudit-variants seed 2; claude graph-only locate-agentruntime
  seed 2; codex no-graph trace-policy-denial-wiring seed 2).

---

## Reviewer role for Claude's outputs

When Claude pings with synthetic Rust modules or harness extensions, please
review with the same discipline you applied to the v4 design draft. In
particular:

- **Synthetic modules** (`benchmarks/graph/v4/_fixture_code/*.rs`): check that
  symbol names don't collide with production names (BFS-by-name pollution
  rule). The empirical demo of this is in `_fixture_code/callers_2hop_graphbenchpolicy.rs`
  (the `resolve` → `lookup_rule` rename in commit `af8114b0`).
- **Harness extensions** (`benchmarks/graph/scripts/oracle.py`,
  `aggregate.py`): check that v1/v2/v3 records still parse with no behaviour
  change. v4 is supposed to be backward-compatible with prior rounds.

---

## References

- **Design doc:** `benchmarks/graph/v4/METHOD.md`
- **Synthetic-module exemplar:** `benchmarks/graph/v4/_fixture_code/callers_2hop_graphbenchpolicy.rs`
- **Closing entry on the v1–v3 series:** `docs/design/knowledge-graph/5_null_result.md`
- **Knowledge-graph design (precision-gap rationale):** `docs/design/knowledge-graph/2_design.md`
  §"Reference resolution"
- **Existing aggregator:** `benchmarks/graph/scripts/aggregate.py`
- **Existing oracle:** `benchmarks/graph/scripts/oracle.py`
- **Convention doc:** `benchmarks/CONVENTIONS.md`

## Coordination

When blocked, write a short note in this file under a new `## Codex notes`
section at the bottom (don't edit the task spec above without proposing a
diff first). Claude monitors this file.

Push back on anything in the schema, prompt directions, or ground-truth
specs above that doesn't match what you find. The session has multiple
precedents for Claude being wrong about production-code shape — review,
don't trust.
