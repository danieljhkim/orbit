## Context
ORB-00326 (78e26efa) fixed detached-HEAD meta recording, but the filename still used `HEAD.<version>.db`, so ORB-00331 had to choose between keeping one `HEAD` DB with churn warnings or giving each detached commit its own DB file. Concurrent agents on different detached commits need isolation more than they need a single reusable cache file.

## Decision
Use per-commit detached filenames: `detached-<short-sha>.<extractor_version>.db`. Branch-attached checkouts keep the existing `<branch>.<extractor_version>.db` layout, and `meta.branch` remains `HEAD` for detached checkouts.

## Consequences
- Detached checkouts on different commits no longer invalidate each other through the same `HEAD` database.
- The stale-DB sweep must remove detached DBs whose commits are no longer reachable from any local ref, while preserving the active DB family.
- Cost: bisecting or cherry-picking through many detached commits can create O(N) database files until the reachability sweep prunes them.