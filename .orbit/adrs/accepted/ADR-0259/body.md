**Context.** Dashboard shipment reproduced the same provider-launcher ENOENT previously seen in routine sweeps: independent process entry points inherited different `PATH` values even though every `backend: cli` provider invocation converges on one engine spawn boundary. The alternatives were to keep pinning `PATH` in each service/entry point or resolve configured launcher names once at that shared boundary.

**Decision.** Resolve every bare provider launcher at the orbit-engine CLI spawn boundary. Search the process `PATH` first, then portable per-user fallback directories derived from `HOME`; preserve explicitly pathed commands unchanged. Missing-launcher failures remain permanent and name the provider plus every searched path.

**Consequences.**
- Dashboard, routine, CLI ship, and direct job dispatch share one provider-launcher resolution mechanism rather than depending on each parent environment being curated.
- Explicit command paths and `PATH` precedence remain authoritative, while common user-local installations work from scrubbed service environments.
- Cost: Orbit now recognizes a small ordered set of conventional user-local bin directories outside `PATH`, so moving a launcher into a new convention requires extending and testing that list.