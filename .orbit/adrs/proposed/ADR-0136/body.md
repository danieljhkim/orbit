## Context
The sync coordinator needs in-process control over fetch, commit, and push: typed errors, programmatic auth callbacks, and the ability to retry without subprocess overhead. Two viable options: the `git2` crate (libgit2 bindings) or shelling to the system `git` binary. Shelling is simpler to reason about — you get exactly what `git` does — but error handling is brittle (stdout parsing) and auth integration with credential helpers requires reimplementing git's helper protocol.

## Decision
The sync coordinator uses `git2`. Auth callbacks integrate with `git_credential_helper` directly; errors are typed; retries are in-process; the coordinator can hold an open libgit2 handle for the duration of a session.

## Consequences
- In-process operation; no subprocess overhead per mutation.
- Auth integrates with system credential helpers via libgit2's existing callbacks.
- Cost: `git2` has a steeper learning curve, occasional ABI churn between releases, and a larger binary footprint than the standalone Orbit binary today. The crate is well-maintained but adds a non-trivial native dependency.

---
