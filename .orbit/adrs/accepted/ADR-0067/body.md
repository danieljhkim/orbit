## Context
Every read can trigger `ensure_fresh`. Without coordination, a dirty worktree plus many quick reads would stack rebuilds, and concurrent callers would duplicate work.

## Decision
Guard rebuilds with a `flock` on `refresh.lock`. Debounce dirty-worktree rebuilds against a fingerprint + timestamp in `refresh_state.json` (default 5s, `ORBIT_KNOWLEDGE_REFRESH_DEBOUNCE_SECS`). Freshness also requires the current branch ref to exist, so debounce cannot suppress the first build for a missing branch ref. Concurrent callers wait briefly for the in-flight rebuild rather than starting their own.

## Consequences
- Steady-state read cost on a dirty worktree is one rebuild per debounce window, not one per read.
- Corrupt-store recovery path ([T20260416-0719]) lives in the same critical section.
- Cost: the first reader after a change pays full rebuild latency; subsequent readers ride the cache.

---
