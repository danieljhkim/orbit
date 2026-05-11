## Context
Incremental rebuilds already computed `ctx.changed_paths`, but the leaf phase still re-read and re-extracted every extractable file. That made dirty-read refreshes O(repo) even when one file changed, and it wasted the content-addressed store's ability to preserve identical file/leaf objects.

## Decision
During incremental builds, `build_graph_leaves` reads the previously persisted graph for the same branch ref and reuses unchanged file snapshots when the file source hash and every reused leaf's `file_hash_at_capture` match the new hash. Changed paths, new files, hash mismatches, absent refs, and unreadable prior graphs fall back to the normal extractor path; directory and file skeletons are still rebuilt from the current scan so deletes and ignore-rule changes are reflected.

## Consequences
- Single-file edits reduce extraction work from the whole repo to the changed path set.
- Zero-change incremental rebuilds can reproduce the previous root object hash byte-for-byte because reused leaves preserve IDs, identity keys, and source hashes.
- Deleted or newly ignored files naturally disappear because reuse only considers files in the current scan.
- Cost: extractor improvements do not automatically reparse unchanged files during an incremental rebuild; users need a full `orbit graph build` when extractor semantics, not file contents, are what changed.

---
