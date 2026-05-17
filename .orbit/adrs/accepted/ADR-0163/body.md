## Context

`orbit.design.init` accepts a feature name and scaffolds a folder. The question was whether to overwrite an existing folder (with or without a `--force` flag), error on existing, or merge into existing (write only the missing files). Overwrite is convenient but destructive — a re-run after editing would silently undo the author's work. Merge is convenient but produces an inconsistent state when a folder is partially scaffolded by hand and partially by tool.

## Decision

`init_feature` errors with a typed `InvalidInput` when the target folder already exists. There is no `--force` or `--merge` flag. To re-scaffold, the author must move or delete the existing folder explicitly.

## Consequences

- A re-run after editing cannot silently destroy work.
- The init operation has a clean precondition (folder absent); diagnosis of failures is a single check.
- Cost: an author whose first scaffold was a typo (e.g. they ran `init` with the wrong feature name, then noticed) has to delete the wrong folder before re-running. There is no in-tool fix. This has not been painful in practice — typos surface immediately because the response shows the path — but the lack of a `--force` is occasionally requested.