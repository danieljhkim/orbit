---
summary: "Worktree Artifacts - Design"
type: design
title: "Worktree Artifacts - Design"
owner: codex
last_updated: 2026-08-11
status: Accepted
feature: worktree-artifacts
doc_role: design
tags: ["worktree-artifacts"]
paths: ["crates/orbit-remote/**", "crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-cli/**"]
related_features: ["worktree-artifacts", "host-registry", "mcp-bridge"]
related_artifacts: ["ORB-00199", "ORB-00200", "ORB-00201", "ORB-10272", "ORB-10297", "ORB-10330", "ORB-10545", "ORB-10668", "ORB-10669", "ORB-10725", "ADR-0177", "ADR-0229", "ADR-0302", "ADR-0339", "ADR-0342", "ADR-0357"]
---

# Worktree Artifacts — Design

> Learning-specific storage and federation references below are retired history.
> [ORB-10736] / [ADR-0359] remove the native learning subsystem and leave its
> existing repository files inert.

The current implementation treats ADR and learning bodies as branch-local files with workspace-local IDs ([ADR-0357]). The shared root owns durable coordination state; the local root owns files that should be staged with the branch.

## 1. Runtime Roots

`OrbitRuntime` carries `shared_root` and `local_root`. On the main checkout they are equal. In a linked worktree, `shared_root` points to the main checkout `.orbit/`, and `local_root` points to the linked worktree `.orbit/`.

Explicit `--root` and `ORBIT_ROOT` overrides pin both roots to preserve the old single-root mental model when the operator asks for it.

## 2. Allocation Metadata

In standalone/worktree mode, `id_allocations` lives in
`shared_root/.orbit/state/semantic.db`. The allocator serializes ID creation with a
shared lock, then body writes update the row with:

- `worktree_root`: the recorded worktree root for the body.
- `branch`: best-effort current branch.
- `body_path`: the body file path relative to `worktree_root`.

Backfilled shared-root artifacts receive `body_path` during allocator initialization so old ADRs and migrated learnings remain readable from any worktree.

This is the only allocator, and every create path uses it. [ADR-0357] keys
knowledge `(workspace_id, artifact_key)`, so an ID is unique within its workspace
and makes no claim outside it; [ORB-10725] deleted the hub-global sequence that
§2.1 and §2.2 once described.

### 2.1 The withdrawn hub-global sequence substrate

[ORB-10272] added Remote feature migration v2: dormant hub-global ADR and learning
sequences in the hub's config-resolved `orbit.db`, per-workspace reconciliation
state, an immutable `mcp_call_id` allocation ledger, and a dormant/active authority
marker. [ORB-10330] added the owner-side `finalize_preallocated` paths and the
gated broker composition that paired one hub allocation with one owner-checkout
finalization, correlated by `mcp_call_id`.

**Both are removed** ([ORB-10725], [ADR-0357]). Public issuance never activated, so
no ID was ever drawn from the sequence and nothing had to be renumbered; what the
substrate encoded was a superseded model, which is why it was deleted rather than
parked alongside the registry tables that ADR-0358 keeps dormant for v2. Remote
feature v2 keeps its ledger slot — feature-migration names are immutable, so a
database that recorded it must still find it — but the slot is a no-op, and Remote
feature v3 drops the tables `IF EXISTS` so a database that applied the original v2
converges on the same shape as a fresh one.

What is left is the shape §2 already described: one workspace-local allocator per
workspace, reached only by the owning machine. `orbit.learning.add` and
`orbit.adr.add` are single owner-local transactions — no reservation, no expiry, no
orphaned ID, no finalize/pull race — and the [ORB-10364] authoring role gate sits on
that one surface with nothing at stake on refusal.

## 3. Write Path

ADR creation writes `adr.yaml` and `body.md` under `local_root/adrs/proposed/ADR-NNNN/`. Learning creation writes `learning.yaml`, `votes.jsonl`, and `comments.jsonl` under `local_root/learnings/L-NNNN/`.

The first write into a linked worktree creates only the subtree needed for the artifact type. It does not scaffold local `state/`, `audit/`, `tasks/`, scoreboards, or registry files.

## 4. Read Federation

ADR `show` resolves the envelope and body together in the store and carries exactly one of four states through Core, HTTP, and local MCP:

1. `local`: a structurally complete, non-empty bundle exists under the current local root. This wins over any readable sibling allocation, and the read does not rewrite the allocation row.
2. `federated`: no current-local bundle exists and the allocation resolves to a structurally complete, readable sibling bundle.
3. `remote_artifact_unavailable`: an allocation exists, but its `body_path`, `adr.yaml`, or `body.md` is absent, unreadable, inconsistent, or empty.
4. `not_found`: neither a current-local bundle nor an allocation exists.

Successful local and federated reads preserve the ADR envelope, return the exact non-empty `body`, and add:

```json
{
  "artifact_origin": {
    "mode": "local | federated",
    "worktree_root": "credential-safe path",
    "branch": "string or null"
  }
}
```

`mode` has no other values. Public origin payloads never include `body_path`, URLs, bearer material, or credentials. HTTP maps `remote_artifact_unavailable` to 409 while retaining its string `error`; local MCP uses the same stable code in its structured error. An unknown ID remains HTTP 404 / MCP `not_found`.

List and search retain their existing defaults. Readable allocation-owned bundles participate as before; unreadable rows are omitted unless `include_remote` requests the existing stub shape.

## 5. Mutation Boundary

ADR document update, accept, and supersede are local-only. A federated or unavailable allocation-owned artifact fails preflight with `artifact_not_local` (HTTP 409 or the same local MCP code) before any bundle, allocation, lifecycle timestamp, index, or audit mutation. Supersede preflights both operands before its first write. Landing the bundle in the current checkout restores ordinary local mutation semantics; a sibling-owned allocation row remains unchanged.

Local-only is a boundary on *where* the write runs, not on which surface may run
it. `orbit adr add`, `orbit adr update`, and `orbit adr supersede` ([ADR-0342],
[ORB-10668]) delegate to the same `orbit.adr.*` tools and so inherit this
preflight unchanged; they exist so the checkout that owns a bundle can complete
the lifecycle from a shell, which is the only place the 409 leaves open.

### 5.1 Federated ADR reconciliation

`orbit adr reconcile <id> --source-worktree <path>` localizes an already
published ADR bundle from an explicitly named registered sibling worktree. It
does not allocate, reconstruct metadata, change lifecycle state, or repoint the
allocation row. The store requires both checkouts to appear in the same Git
worktree registry, reads a complete non-empty bundle whose metadata status
matches its lifecycle partition, and accepts an existing destination only when
both files are byte-equivalent.

The source and destination ADR locks cover validation, while the shared
allocator lock pins the complete allocation snapshot across the final atomic
rename. A changed source, allocation, incomplete bundle, lifecycle mismatch,
unregistered checkout, or divergent destination fails before the canonical
destination changes. This is deliberately distinct from `adr restore`: restore
authors a replacement only when no readable copy exists; reconciliation
preserves a readable published bundle verbatim.

## 6. Publication Policy and Duplicate Partitions

Every ADR lifecycle partition — `proposed/`, `accepted/`, `superseded/`,
`deleted/` — is tracked by git and travels with the repository [ORB-10669].
Publishing `proposed/` puts the decision under review in the same PR as the
change that motivates it, and means an ADR authored inside a managed job
worktree lands on that run's branch instead of stranding on the box until an
operator reconciles it. Only the rebuildable `adrs/index.sqlite*` and the
host-local `*.lock` files stay ignored. The managed `.gitignore` block that
`orbit workspace init` writes carries this policy to every workspace; re-init
over an older block retires that block's `proposed/` and `superseded/` ignore
lines rather than leaving them to out-rank the appended re-include.

Publishing every partition makes a duplicate ID reachable. Acceptance is a
directory rename, so a branch cut before acceptance still carries
`proposed/<id>`; merging it re-adds that directory next to `accepted/<id>`, and
git merges both without a conflict because the two paths are unrelated.
Resolution follows one explicit precedence: **the most-advanced lifecycle state
wins** (`proposed` < `accepted` < `superseded` < `deleted`), because every
sanctioned transition moves forward and `accepted → proposed` is rejected
outright — so the lower-ranked copy is always the stale one. `get_adr`, every
mutation preflight, and `list_adrs` share that precedence, and each shadowed
partition is named in a `warn` log with its path, so the leftover is visible and
removable rather than silently deciding the read. This replaces the implicit
precedence that fell out of partition declaration order. `orbit adr reconcile`
keeps its stricter contract: a source checkout holding more than one lifecycle
artifact for an ID is refused, not resolved.

### 6.1 Managed job-worktree drafts

A managed job-run worktree scaffolds a real `.orbit/adrs/proposed` for its run.
Drafts written there are now tracked, so the on-box auto-commit sweeps them into
the run's branch. That is the intended disposition for work that ships: the
draft rides its PR and merges with the code. For a run that is abandoned or
whose PR is rejected, the disposition is that **the draft dies with the branch**
— no operator cleanup step, no reconciliation. Nothing reaches `agent-main`
unless the branch merges, and the ID allocation left behind is an ordinary valid
gap (§2). Deleting the worktree removes the only copy; that is expected and is
not the orphaned-allocation condition ORB-10501 repairs. An operator who instead
wants to keep a draft from a discarded branch pulls it over with
`orbit adr reconcile` before the worktree is reaped.

## 7. Indexing Behavior

Learning reindex and docs/ADR search operate on locally readable bodies. Remote-only allocation rows are skipped without error; once the recorded worktree is present and readable again, the same list/reindex path can read and index the body.

## 8. Concerns & Honest Limitations

Remote stubs are intentionally envelope-poor. They expose allocation metadata, not the artifact title, summary, or body, because those fields live in the unreadable body file. Filters that require body fields can only apply to locally readable artifacts.

The `worktree_root` column preserves historical rows from earlier phases, so old shared-root rows may record a `.orbit/` path while new rows record a worktree root. Readers resolve `body_path` relative to the recorded value instead of normalizing that history away.

## Task References

- [ORB-00199] introduced the runtime root split.
- [ORB-00200] introduced allocation metadata and the learning ID migration.
- [ORB-00201] implemented local body writes and read federation.
- [ORB-10297] made ADR federation body-preserving and typed the read/mutation boundary.
- [ORB-10272] added the dormant, path-free Remote-v2 hub sequence and reconciliation
  substrate while preserving the standalone shared-root allocator and owner-local
  body/federation semantics. Never activated; removed by [ORB-10725].
- [ORB-10330] added the owner-side preallocated finalizers (`finalize_preallocated`
  on the ADR and learning stores) and the gated broker composition. Removed by
  [ORB-10725]: with no allocation step there is no preallocated ID to finalize.
- [ORB-10725] deleted both substrates under [ADR-0357], turned Remote feature v2
  into a no-op slot, and added Remote feature v3 to drop its tables from databases
  that had applied it.
- [ORB-10545] added exact-bundle reconciliation and made superseded ADR bodies
  repository-published decision history under [ADR-0302].
- [ORB-10669] published the remaining partitions (§6) under [ADR-0339], made the
  managed `.gitignore` block retire its own superseded lines, and replaced
  first-hit-wins ADR resolution with the explicit lifecycle precedence.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
