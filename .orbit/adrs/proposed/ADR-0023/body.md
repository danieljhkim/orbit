## Context
ADRs as v2-ships-them have no formal review surface. Comments on a proposed ADR happen informally — in the related task's review threads, in PR discussion, or in chat. Tasks already have a structured review-thread surface (`orbit.task.review_thread.*`); the question is whether ADRs warrant the same.

## Decision
Yes. ADRs get `orbit.adr.review_thread.{add, list, reply, resolve}`, mirroring the task surface. Threads are scoped to a single ADR by `adr_id`. Whether the `proposed → accepted` transition should require all threads be resolved is a follow-up question deferred until the surface has real use — for now, the transition does not block on thread state.

## Consequences

- Reviewers comment in a structured surface specific to the decision being reviewed, not buried in a task that may cover several decisions.
- Resolution state is queryable: `orbit.adr.review_thread.list --status=open` surfaces unresolved feedback across the corpus.
- Schema and tool surface grow to accommodate threads. Storage attaches threads under the per-ADR directory (`.orbit/adrs/<status>/<id>/review_threads/`), keeping the directory-per-ADR pattern intact.
- Cost: four new tools to maintain and document; risk of duplicating discussion across both task-level and ADR-level review threads when a task implements exactly one ADR. Mitigation: CONVENTIONS.md guidance at v2 ship time — ADR review threads for *the decision*, task review threads for *the implementation*. If duplication remains a real problem post-ship, consider auto-linking the two thread surfaces.

---
