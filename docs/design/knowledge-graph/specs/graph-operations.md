# Spec: Graph Operations

**Obsolete attribution note ([T20260506-11]).** This spec predates removal of graph task attribution. Its `task_ids`, `attribute.rs`, and matcher-consumer language is historical; there is no current read-side hook that consumes these operation logs for attribution. The symbol-operation taxonomy may still inform future graph write tooling, but it needs a fresh consumer and migration design before implementation.

Orbit-owned symbol-level write operations were proposed as the precise-by-construction complement to the historical identity matcher. When an agent dispatched by Orbit edits code through a graph-aware API (`graph.rename`, `graph.move`, `graph.split`, ...) the commit that carries those edits also carries a sidecar `.orbit/operations/<commit_sha>.json` file. The file enumerates the exact set of symbol-level changes the producer intended. In the original design, the knowledge-graph rebuilder consumed this sidecar to preserve `task_ids` and stable identity *by fiat*, falling back to the matcher only for commits without a log or with a log that contradicts the tree. The task-attribution consumer was removed in [T20260506-11].

## Why This Exists

[T20260421-0528] shipped `task_ids` attribution with an identity-only matcher that keys nodes on `(path, qualified_name, kind)`. That matcher is correct for the steady state and for non-Orbit writes (manual edits, upstream merges, external PRs). It is lossy on any refactor that changes those three fields: a rename, a cross-file move, or a symbol split. Under the matcher, such refactors present as a deletion plus an unrelated creation; the successor node starts with empty `task_ids` and history appears to end at the rename hop. The same file at [2_design.md §6.3](../2_design.md) names this as the accepted limitation.

The long-term direction aligned with the owner is that **Orbit itself performs the refactors** — agents do not wield raw file edits, they call symbol-level primitives, and the commit those primitives produce carries an explicit operation log. Under that model rename and move preserve identity by construction, body-similarity heuristics stay dropped permanently (not because we chose lossy, but because we chose precise), and `task_ids` propagation is exact for any refactor Orbit performs. The matcher remains the reconciliation layer for writes Orbit did not author. Operations enrich; they do not replace.

[T20260421-0528] reserved the read-side hook at `crates/orbit-knowledge/src/pipeline/attribute.rs` (around line 96). Today that hook accepts a permissive skeleton — a file with `{"operations": []}` short-circuits the matcher for the commit. This spec defines the full schema that hook will validate against when the producer exists.

## 1. Operation Taxonomy

The minimum set that preserves identity across every refactor shape Orbit intends to own. Each operation is **atomic per entry**: one entry touches one logical subject (or one N-ary grouping for split/merge). Operations that change multiple axes at once (e.g. move + rename) are expressed as **two entries** referencing the same `stable_id` — no compound "relocate" primitive. This keeps per-entry tree-diff validation simple and keeps the vocabulary minimum small enough to fit in a producer API without overlap.

| Op                 | Subject  | Definition                                                                                                              | Required inputs                                            | Outputs (effect on graph state)                                                 | Compositional? |
|--------------------|----------|-------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------------|----------------------------------------------------------------------------------|----------------|
| `create`           | 1 symbol | A symbol appears that had no predecessor in the prior build. The producer claims the new symbol as its own (not an extraction). | `post` address; `body_hash_post`; optional `signature_post` | Fresh `stable_id` allocated and persisted on the new node.                       | Atomic |
| `delete`           | 1 symbol | A symbol present in the prior build no longer exists post-commit. Not an extraction target. | `stable_id`; `pre` address snapshot (for validation)        | `stable_id` is retired; no successor node.                                       | Atomic |
| `rename`           | 1 symbol | `qualified_name` changes; `file` and `kind` are unchanged. | `stable_id`; `pre.qualified_name`; `post.qualified_name`    | `stable_id` preserved; `identity_key` recomputed from the new `qualified_name`.  | Atomic |
| `move`             | 1 symbol | `file` changes; `qualified_name` and `kind` are unchanged. | `stable_id`; `pre.file`; `post.file`                        | `stable_id` preserved; `identity_key` recomputed from the new `file`.            | Atomic |
| `change_signature` | 1 symbol | Public-facing signature changes (arg list, return type, generics). `qualified_name`/`file`/`kind` unchanged. | `stable_id`; `signature_pre`; `signature_post`              | `stable_id` and `identity_key` preserved; stored `signature` and likely `body_hash` updated. | Atomic |
| `change_body`      | 1 symbol | Body content changes; signature and address unchanged. | `stable_id`; `body_hash_pre`; `body_hash_post`              | `stable_id` and `identity_key` preserved; stored `body_hash` updated.            | Atomic |
| `split`            | N from 1 | One symbol becomes N symbols; the original `stable_id` is retired. Each successor is independently addressable. | `source.stable_id`; `children[]` (each with `post` address and `stable_id`) | Source `stable_id` retired; N fresh `stable_id`s allocated (one per child), each persisted on a new node. | Atomic (N-ary) |
| `merge`            | 1 from N | N symbols become 1; N−1 `stable_id`s are retired and one is preserved (or a fresh one allocated) as the target. | `sources[].stable_id`; `target.stable_id`; `target` address | All source `stable_id`s except `target.stable_id` retired; target node persisted under `target.stable_id` (either inherited from a source or freshly allocated). | Atomic (N-ary) |

Notes:

- **Compound refactors** (move-and-rename, rename-and-change-body) emit two entries. The producer is responsible for ordering them deterministically — canonical order is the column order of this table.
- **`change_body` is optional.** A commit that only mutates a body can omit the entry; the matcher attributes the change the same way it does for non-Orbit writes. This keeps the sidecar small for the common case. The producer MAY emit `change_body` when it wants explicit credit, but the consumer treats its presence as informational, not load-bearing. See §7 (Q3) for the drift concern.
- **`create` does not cover extractions.** A new symbol that is the product of a `split` is represented only in the `split` entry's `children[]`, not in a separate `create`. The producer distinguishes between "this symbol is genuinely new" (→ `create`) and "this symbol came from splitting an existing one" (→ `split`).

## 2. Canonical Schema

The sidecar lives at `.orbit/operations/<commit_sha>.json` — sibling to `.orbit/knowledge/`, not nested under it. One file per commit. The path is deliberately commit-scoped (not branch-scoped) because operations describe what a commit *did*, not what a branch looks like.

### 2.1 Envelope

```jsonc
{
  "schemaVersion": 1,
  "commitSha": "abcd1234...40-hex...",
  "producer": {
    "name": "orbit-engine",          // required — who wrote the log
    "version": "0.1.0",              // required — producer build version (semver)
    "agent": "claude",               // optional — agent identity when applicable
    "model": "claude-opus-4-7"       // optional — model identity when applicable
  },
  "operations": [ /* tagged union, §2.2 */ ]
}
```

- `schemaVersion`: monotonically increasing integer. Readers refuse to short-circuit the matcher when `schemaVersion > supported`.
- `commitSha`: 40-character lowercase hex. MUST match the file-name stem. Mismatch is malformed.
- `producer.name`, `producer.version`: required. Enables future per-producer consumer policy (e.g. "trust orbit-engine ≥ 0.5.0 for split/merge; warn otherwise").
- `producer.agent`, `producer.model`: optional; populated when the edit went through an agent dispatch loop. Intended for auditing, not for consumer branching logic.

### 2.2 Per-operation shape (tagged union)

Every operation entry uses `op` as the discriminator. Fields outside the `op`-specific table below are rejected on strict validation (extension is additive and future-versioned).

Common fields (all entries):

| Field       | Type              | Required | Notes |
|-------------|-------------------|----------|-------|
| `op`        | string            | yes      | One of the eight op kinds. |
| `note`      | string            | no       | Free-form producer comment; informational only. |

Per-op required fields:

```jsonc
// create
{ "op": "create",
  "stable_id": "node:<nanoid-21>",
  "post": { "file": "...", "qualified_name": "...", "kind": "..." },
  "body_hash_post": "<sha256-hex>",
  "signature_post": "..." /* optional */ }

// delete
{ "op": "delete",
  "stable_id": "node:...",
  "pre":  { "file": "...", "qualified_name": "...", "kind": "..." } }

// rename
{ "op": "rename",
  "stable_id": "node:...",
  "pre":  { "qualified_name": "..." },
  "post": { "qualified_name": "..." } }

// move
{ "op": "move",
  "stable_id": "node:...",
  "pre":  { "file": "..." },
  "post": { "file": "..." } }

// change_signature
{ "op": "change_signature",
  "stable_id": "node:...",
  "signature_pre":  "...",
  "signature_post": "..." }

// change_body
{ "op": "change_body",
  "stable_id": "node:...",
  "body_hash_pre":  "<sha256-hex>",
  "body_hash_post": "<sha256-hex>" }

// split
{ "op": "split",
  "source":   { "stable_id": "node:...",
                "pre": { "file": "...", "qualified_name": "...", "kind": "..." } },
  "children": [
    { "stable_id": "node:...",
      "post": { "file": "...", "qualified_name": "...", "kind": "..." },
      "body_hash_post": "..." }
    /* 1..N */
  ] }

// merge
{ "op": "merge",
  "sources": [ { "stable_id": "node:...",
                 "pre": { "file": "...", "qualified_name": "...", "kind": "..." } }
               /* 2..N */ ],
  "target":  { "stable_id": "node:...",     // may equal one of sources[].stable_id
               "post": { "file": "...", "qualified_name": "...", "kind": "..." },
               "body_hash_post": "..." } }
```

`pre` / `post` address snapshots include only the fields an operation changes plus enough context for validation. For `rename`, `file` and `kind` are invariant and therefore redundant — they are looked up from the pre-commit graph by `stable_id`. Producers MAY include them for defensive validation; the consumer tolerates but does not require them.

Full JSON Schema (formal) will be committed under `crates/orbit-knowledge/src/pipeline/schemas/operations-v1.json` when the producer task lands. This spec's prose is authoritative until that file exists.

## 3. Symbol Addressing — Stable IDs

**Decision:** operations address symbols by a graph-level stable identifier persisted on each node. The alternative — pre-op/post-op address pairs — was considered and rejected; see §3.3.

### 3.1 Shape and allocation

A new field `stable_id: String` is added to `BaseNodeFields` ([crates/orbit-knowledge/src/graph/nodes.rs](../../../crates/orbit-knowledge/src/graph/nodes.rs)). Format:

```
node:<nanoid-21>
```

Content-free by design. Random 21-character nanoid (126 bits of entropy; collision risk is negligible at any repo scale Orbit will plausibly encounter). The `stable_id` is NOT derived from any node content, so renames, moves, signature changes, and body changes all leave it unchanged.

Allocation happens in three paths, in priority order:

1. **Operation log claim.** If the commit's operation log names a `stable_id` in a `rename`/`move`/`change_*`/`split.children[]`/`merge.target` slot, the rebuilder assigns that exact ID to the successor node. The producer is authoritative for identity.
2. **Identity matcher inheritance.** When no operation log covers a symbol, the matcher (T0528) finds a predecessor by `(path, qualified_name, kind)` and carries its `stable_id` forward. This handles non-Orbit writes unchanged.
3. **First sight.** If neither source provides an ID, the rebuilder allocates a fresh `node:<nanoid-21>` and persists it on the node.

### 3.2 Persistence and durability

`stable_id` is part of the node JSON body under content-addressed storage (ADR-001). Two consequences:

- **Object hash depends on `stable_id`.** Two otherwise-identical nodes with different `stable_id`s hash to different object files. Dedup (ADR-001) still works for same-content nodes — the same body under the same `stable_id` across rebuilds produces the same object hash — but it does not collapse across *independently-allocated* IDs (e.g. two clones of the same repo that each ran first-sight allocation separately).
- **Orphan accumulation.** Re-allocating a `stable_id` (not expected in normal flow) produces a new object hash and leaves the old one orphaned. GC remains an open question ([3_vision.md §1.10](../3_vision.md)).

Stable IDs are not surfaced at the query API level today — all reads go through selectors. `stable_id` is producer/consumer vocabulary only. A future addition of a by-id lookup is trivial but out of scope for this spec.

### 3.3 Why stable IDs over pre/post address pairs

Two reasons that compound:

1. **Disambiguation under simultaneous axis changes.** A commit that both renames and changes the body of the same symbol would, under address-pair encoding, require the consumer to decide whether two separate entries (`rename` + `change_body`) refer to the same underlying symbol or two different refactors that happen to chain. With stable IDs, both entries carry the same `stable_id` and the question does not arise. Address-pair encoding would need an auxiliary correlation key — which is a stable ID under a different name.
2. **N-ary operations need named subjects.** `split` has one source and N children; `merge` has N sources and one target. Naming them by address pairs requires nested disjoint-union gymnastics and ambiguous validation ("which source became which child of the split?"). Stable IDs give each subject a handle, and the consumer reads the operation exactly as written.

Third reason, weaker but worth naming: stable IDs survive producer-internal bookkeeping (working-graph edits that chain across an activity's turn) with the same shape the on-disk graph uses. Address pairs would require the producer to maintain a parallel mapping.

### 3.4 Migration

First rebuild after the producer ships: every existing node is treated as "first sight" and allocated a fresh `stable_id`. This is a one-time schema bump. Historical object hashes change; old object files become unreachable from the new root. This is tolerated because (a) the graph is rebuildable from the repo, (b) no external consumer pins historical object hashes, and (c) GC is already an open item.

## 4. Validation and Divergence Policy

The operation log is **advisory-authoritative**: accepted when consistent with the tree diff, ignored when inconsistent. The consumer never rejects a commit or blocks the rebuild on the basis of a malformed log. Maximum impact of a bad log is that the commit falls back to the matcher (the pre-producer behavior).

### 4.1 Well-formed

A log is well-formed when all of these hold:

1. Valid JSON (UTF-8, parses).
2. Envelope fields (`schemaVersion`, `commitSha`, `producer.name`, `producer.version`, `operations`) present with correct types.
3. `schemaVersion ≤ consumer.supportedVersion`.
4. `commitSha` matches the file-name stem (lowercase 40-hex).
5. Every entry has a recognized `op` discriminator and the required fields for that discriminator.
6. No unknown fields at the per-op level (strict; extension is additive and version-gated).

Violations are **malformed**. Behavior: emit a stderr warning and fall back to the matcher for that commit (the pre-producer default). This matches the existing permissive-stub behavior in `inspect_operation_log`.

### 4.2 Semantically valid against the commit's tree diff

Even a well-formed log may lie. A `rename(stable_id=X, pre.qualified_name="Foo", post.qualified_name="Bar")` is valid only if the commit's tree diff shows `Foo` absent from the post-tree and `Bar` present. Per-op tree-diff checks:

| Op                 | Must hold in tree diff |
|--------------------|------------------------|
| `create`           | `post` address present post-commit, absent pre-commit. |
| `delete`           | `pre` address present pre-commit, absent post-commit. |
| `rename`           | Symbol at `pre.qualified_name` gone; `post.qualified_name` present; `file`/`kind` match. |
| `move`             | Symbol at `pre.file` gone; `post.file` present; `qualified_name`/`kind` match. |
| `change_signature` | Address unchanged; signature differs. |
| `change_body`      | Address and signature unchanged; body hash differs. |
| `split`            | `source` address absent post-commit; all `children[]` addresses present post-commit. |
| `merge`            | All `sources[]` addresses absent post-commit; `target` address present. |

### 4.3 Divergence policy

| Case                                                | Policy                      |
|-----------------------------------------------------|-----------------------------|
| Malformed (§4.1 violation).                         | Warn, fall back to matcher. |
| Well-formed, op inconsistent with tree diff.        | Warn, fall back to matcher **for that commit** (all ops in the log are discarded together — partial acceptance is out of scope). |
| Op references a `stable_id` unknown in the pre-build and not defined earlier in the same log. | Warn, fall back to matcher. |
| Log is well-formed and every op is consistent, but the tree shows additional symbol-level changes not covered by any op. | Accept: ops cover what the producer claimed; residual changes flow through the matcher as normal. Explicit coverage and implicit inference coexist. |
| `schemaVersion` exceeds consumer support.           | Warn, fall back to matcher. No attempt to partially interpret. |

Rationale for "prefer tree over operation": the tree is the ground truth for what the commit did. A producer bug that claims a rename not reflected in the tree would, if trusted, silently rewrite history. The asymmetry is deliberate — the operation log can only enrich, never override.

## 5. Producer Surface Sketch

The agent-facing API lives in `orbit-engine` (tentative). Not a full spec — examples per op showing the call signature and the JSON payload it emits. Agents never write the sidecar directly; the engine buffers operations per working-graph session and flushes them alongside the commit.

```rust
// create
graph.create(post: Address, body: &str, signature: Option<&str>) -> StableId
// emits:
// {"op":"create","stable_id":"node:abcDEF...","post":{...},"body_hash_post":"sha256:..."}

// delete
graph.delete(stable_id: StableId)
// emits:
// {"op":"delete","stable_id":"node:...","pre":{...}}

// rename
graph.rename(stable_id: StableId, new_qualified_name: &str)
// emits:
// {"op":"rename","stable_id":"node:...","pre":{"qualified_name":"old"},"post":{"qualified_name":"new"}}

// move
graph.move_(stable_id: StableId, new_file: &Path)
// emits:
// {"op":"move","stable_id":"node:...","pre":{"file":"a.rs"},"post":{"file":"b.rs"}}

// change_signature
graph.change_signature(stable_id: StableId, new_signature: &str)
// emits:
// {"op":"change_signature","stable_id":"node:...","signature_pre":"...","signature_post":"..."}

// change_body
graph.change_body(stable_id: StableId, new_body: &str)
// emits (optional):
// {"op":"change_body","stable_id":"node:...","body_hash_pre":"...","body_hash_post":"..."}

// split
graph.split(source: StableId, children: &[SplitChild]) -> Vec<StableId>
// emits:
// {"op":"split","source":{"stable_id":"node:...","pre":{...}},
//  "children":[{"stable_id":"node:...","post":{...},"body_hash_post":"..."}, ...]}

// merge
graph.merge(sources: &[StableId], target_address: Address, target_body: &str) -> StableId
// emits:
// {"op":"merge","sources":[{"stable_id":"node:...","pre":{...}}, ...],
//  "target":{"stable_id":"node:...","post":{...},"body_hash_post":"..."}}
```

Binding conventions:

- Every call returns or accepts `StableId`, never raw address pairs. The working-graph layer maintains the mapping from `StableId` to the current in-flight address.
- Compound refactors are two calls: `graph.move_(id, new_file); graph.rename(id, new_name);`. The engine buffers both and emits them as two entries under the same commit's log, preserving order.
- Failed ops (e.g. `rename` to a name already present) raise a `WriteError` and are not buffered. Partial-success semantics are the working-graph's problem, not the operation log's.

## 6. Backfill and Migration

**No retroactive operation logs.** Commits authored before the producer shipped are handled by the identity matcher (T0528) as today. The consumer does not emit or synthesize logs for historical commits.

Rationale: backfill would require running the matcher over history, emitting one-op-per-commit logs claiming to describe what happened. Because the matcher is lossy on renames and moves, those logs would encode that lossiness *as if it were explicit producer output*. That is strictly worse than the current state — it dresses up the matcher's best guess in the clothes of ground truth. The right shape is: historical commits → matcher only; new commits → operation log (when a producer was involved) + matcher (for anything the log doesn't cover).

**Producer rollout.** When the producer lands:

1. Schema bump: `stable_id` added to every node body. First rebuild allocates fresh IDs for all existing nodes.
2. Consumer already tolerates a missing operation log ([T20260421-0528] reserved the hook). No changes needed to the consumer until the producer emits a non-empty log.
3. Consumer upgrade happens before producer enablement: readers must support `schemaVersion: 1` at least at the `operations: []` level before any producer writes a non-empty log.

**Forward compatibility.** `schemaVersion` is monotonically increasing. Readers older than the producer's schema version fall back to the matcher for any commit whose log claims a higher version. No partial parsing across major versions.

**Mixed history.** A repository with producer-era and pre-producer commits mixed in its history has:

- Pre-producer commits: no log file. Consumer takes the matcher path (Phase A/B in `attribute.rs`).
- Producer commits: log file present and well-formed. Consumer takes the log path, matcher is short-circuited for those commits.

Both paths write into the same `task_ids` union on the same node — they are not separate attribution streams.

## 7. Open Questions

### Q1 — Language-agnostic symbol addressing

The `qualified_name` format is language-specific: `a::b::c` (Rust), `a.b.c` (Python), `a/b/c` (Go module paths). This spec stores the raw per-language form in `pre`/`post` fields. A future multi-language producer may need a normalized form to reason about refactors across language boundaries (e.g. a rename in Rust that must be mirrored in a generated Python binding). Decision deferred until a concrete multi-language producer surface exists. Workaround until then: producers emit the native form; cross-language consistency is the caller's problem.

### Q2 — Multi-symbol atomic operations

The current taxonomy represents each entry as either single-subject (create/delete/rename/move/change_*) or bounded-N-ary (split/merge). A transactional refactor that touches unrelated symbols across unrelated files (e.g. "update every call site when this function's signature changes") currently expands to N `change_body` entries plus one `change_signature` — the relationship between them is not modeled. Is a higher-arity `transaction` operation worth having? It would let consumers reason about refactor atomicity, at the cost of a new op kind and a new validation shape. Flag, do not decide.

### Q3 — Optional `change_body` drift

`change_body` is optional (§1). A commit that only mutates bodies may omit all entries, in which case the consumer cannot distinguish "producer intended the body change" from "producer forgot to emit the op". Over time this gap may cause the operation log to under-describe producer-authored commits. Proposed mitigation if drift materializes: require `change_body` for producer-authored commits, make it optional only when the producer is null. Do not land that mitigation today — too speculative without a producer to measure.

### Q4 — `body_hash` authority

For `change_body` and `change_signature`, body and signature hashes could be computed by the producer (what the producer emitted) or by the consumer (what the tree actually contains). When they disagree, is the producer wrong or is the tree ahead of the log? Current spec treats the producer's hash as the claim and validates against the tree diff (§4.2) without naming the hash specifically. This leaves a gap for producer bugs where body_hash_post ≠ actual post-commit body hash. Suggest adding a strict-hash-check option in a future schema version.

### Q5 — Signing and tamper resistance

Explicitly out of scope per the task constraints. If the operation log is ever relied upon outside the same repository's CI, a signing story becomes load-bearing. Not today.

## Agent Signature

Drafted by Claude (`claude-opus-4-7`) on 2026-04-22. Consumer hook reserved by [T20260421-0528]; producer implementation tracked separately.

## Task References

- **[T20260421-0528]** — Historical `task_ids` schema + git history walker; removed by [T20260506-11].
- **[T20260421-0543]** — This spec.
- **[T20260506-11]** — Remove graph task attribution; this spec's attribution consumer is obsolete.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
