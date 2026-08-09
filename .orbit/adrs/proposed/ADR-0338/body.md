## Context
Workspace init keeps `.orbit/adrs/proposed/` gitignored because proposed drafts are local-only until publication, so `git add --all` in the delivery step silently skips a draft documenting the very code being shipped. The implementing agent cannot close the gap itself: in a linked run worktree `.git` points at the main checkout's worktree metadata, which is bound read-only for the sandboxed implementer, so taking `index.lock` fails. Two alternatives were real — un-ignoring the proposed partition (rejected: local-until-publication is a deliberate policy, and it would publish every speculative draft), and leaving the gap (rejected: it either drops the decision or tempts an executor into fabricating an ADR id).

## Decision
The unsandboxed commit step force-stages exactly the ignored `proposed/*/{adr.yaml,body.md}` bundles present in the delivery worktree, before `git add --all`, verifies each landed in the index, and otherwise refuses delivery with a diagnostic naming the bundle and the supported host-side staging path.

## Consequences
- A proposed ADR allocated during a run ships in the same commit as the code it documents, without any change to the gitignore policy or to the accepted and superseded partitions.
- Discovery uses `git check-ignore --stdin`, which answers from ignore rules without locking the index, so it still works when worktree metadata is read-only and can report that condition precisely.
- Cost: delivery now fails closed on an unstageable draft, so a genuinely read-only checkout blocks the commit until an operator stages the bundle host-side; a refused commit is accepted as strictly better than a dropped decision or an invented id.