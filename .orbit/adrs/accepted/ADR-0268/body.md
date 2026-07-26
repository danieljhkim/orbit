## Context

Task-bundle YAML has bumped `schema_version` several times during the v2 rewrite and will keep evolving. Today the read path *rejects* anything that is not exactly `TASK_ARTIFACT_SCHEMA_VERSION`, so any future bump is a hard break — no way to roll forward an older bundle on disk without an ad-hoc one-off script per change. Other artifacts (review threads, artifact manifest, workspace config) carry the same `schema_version` shape and will inherit the same problem.

## Decision

Add `orbit_common::migration` — a tiny framework keyed on `serde_yaml::Value` — and require artifact-owning code to register a `Plan` per lineage. Three opinionated calls baked in:

1. **Untyped (`Value → Value`) steps**, not typed `Vn → Vn+1` transforms. Frequent schema bumps make the typed approach pay the cost of keeping every historical struct alive forever; the untyped chain lets a step be deleted once the version it migrated from is no longer in the wild. Final correctness is enforced by the existing `serde_yaml::from_value::<T>` call after the chain — a broken step fails to deserialize.
2. **Read-time only**, never auto-writes the migrated value back to disk. Auto-writing on read changes mtimes, surprises users, and risks corrupting bundles on a buggy step. Explicit batch on-disk upgrade is out of scope; if it ever becomes needed it lives in a CLI command that calls the same plan.
3. **No rollback.** Most steps are non-reversibly lossy (dropped fields, NOT NULL additions). Forward-fix migrations (write a new step that corrects course) are the documented recovery path.

Each `Plan` is monotonically versioned within a single lineage. Cross-lineage rewrites (e.g. the legacy `T...` task → v2 ORB bundle import) are explicitly out of scope and remain one-shot importers.

## Consequences


- The read path in `v2_bundle::read_bundle_at` goes through `task_migrations::envelope_plan().migrate(...)` before deserializing into `TaskEnvelopeV2`. Today the chain is empty; the next schema bump adds one `add_step(prev, fn)` call.
- A new `OrbitError::Migration(String)` variant carries chain failures distinctly from `OrbitError::Store` so callers (and logs) can tell schema drift apart from IO/parse errors.
- Other artifacts adopt the framework when their owners are ready; nothing is forced. Review-thread metadata and artifact manifest still go through `read_yaml_file` until they need a step.
- Cost: a single `Value` round-trip per envelope read (parse-to-`Value`, then `from_value::<T>`) replaces a direct `from_str::<T>`. Negligible for envelope-sized YAML; benchmark before extending to large lineages.
- Cost: the framework lives in `orbit-common`, the most-depended-on crate. The surface is small (`Plan`, `Step`, `read_schema_version`) and depends only on `serde_yaml` and `OrbitError`, both already in `orbit-common`.

## Provenance

Migrated verbatim from the local heading `task-artifacts/ADR-008` in `docs/design/task-artifacts/4_decisions.md` by [ORB-10458]. Original status line: Accepted · 2026-05 · Forward-only YAML migration framework (`01928e76`)