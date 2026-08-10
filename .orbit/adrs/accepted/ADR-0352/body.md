## Context

Workspace ownership is a declared binding to a machine, never a runtime claim, and the design states the reasoning plainly: coordination has one writer by construction, and two owners for one workspace is split-brain. That held while one operator drove a workspace.

It no longer holds. Several operator sessions can now reach the same workspace concurrently — an off-box orchestrator over the owned tunnel, a local operator broker, a session over SSH — and nothing arbitrates between them. Ownership answers *which machine*, not *which operator*, and two operator sessions on one machine are indistinguishable to the existing model.

The guards that exist do not cover this. The duplicate-dispatch guard is per task id and scans a bounded window of recent runs, so a stale non-terminal run outside the window is invisible. Worse, auto and backlog-discovery submissions carry no task ids at all and are unguarded entirely: two discovery ship runs in one workspace both proceed. Task reservations are file-scoped, advisory, and enforced only at gate-pipeline admission — they arbitrate between workers, not between orchestrators.

The contention is specific. Reading, searching, filing tasks, and authoring knowledge are safe concurrently, and several people working different features in one workspace is the desired behaviour. What cannot be concurrent is *dispatch*: triage, ship, and resume decide what work starts and against which base, and two orchestrators making those decisions independently produce duplicated runs and racing branch state.

## Decision

Introduce an exclusive, TTL'd **workspace claim** held by one operator, and make it a precondition for workflow dispatch only.

- The claim gates exactly the governed workflow operations. Every other operation — task create, read, update, search, knowledge, friction — is unaffected and remains concurrent.
- Enforcement lives at the shared run-submission path, not at a protocol adapter, so every surface inherits it: CLI, HTTP, MCP, and remote command execution alike. A caller holding shell cannot route around it, because the CLI reaches the same chokepoint.
- Acquisition mints a **claim token** returned to the holder and presented on subsequent workflow calls. Machine and session identity are recorded for diagnostics but are not load-bearing, because session identity is minted per connection and does not survive a reconnect.
- Contention is a rejection carrying the current holder and the expiry instant, never a silent queue or a silent steal.
- The claim is TTL-bounded with lazy expiry evaluated on each check, and an explicit force-release exists and is audited.
- Claim scope is a distinct dimension from file reservations. It must not be expressed as a whole-workspace file selector, which would also block the worker reservations it is meant to leave alone.

## As implemented (ORB-10709)

Three choices the decision above left open, each load-bearing:

- **The claim reuses the `task_reservations` table**, separated from worker file reservations by a `scope` discriminator (`files` / `workspace_claim`, schema migration v13) rather than by a parallel table. That inherits the atomic `IMMEDIATE`-transaction acquisition, TTL, lazy expiry, audit, and release escape hatch already built there. *Rejected alternative:* a dedicated `workspace_claims` table — honest about the "distinct dimension", but roughly 250 lines duplicating the machinery this decision exists to reuse. The scope discriminator is a required argument of the shared SQL predicate, so the compiler asks every query site which dimension it reads; claim rows additionally carry an empty file list, so even a forgotten filter cannot produce a path conflict.
- **An unclaimed workspace gates nothing.** The claim arbitrates *between* operators who want one; it is not a mandatory ceremony before every dispatch. Requiring one unconditionally would break every existing unattended dispatch and would make "the refusal names the current holder" meaningless. Refusal happens only when an active claim exists and the caller presents no token or a stale one.
- **`orbit run ship-sweep` stands down rather than failing** when it meets a held claim: the unattended sweep carries no token and does not force, so a claimed workspace is reported as skipped with the holder, not as a sweep error.

Surfaces: `claim_token` on the `orbit.workflow.ship` / `orbit.workflow.run.resume` tools, `orbit run ship --claim-token`, the dashboard ship and resume bodies, and `ORBIT_WORKSPACE_CLAIM_TOKEN` for an operator shell. Acquire / release / force-release / status are the `orbit.workspace.claim.*` tools, registered inactive alongside `orbit.task.locks.*` — operator-reachable through `orbit tool run`, absent from the agent MCP surface. `orbit.workspace.claim.release` is a governed operation requiring `Operator`, because force-release displaces whoever is driving dispatch.

## Consequences

- Concurrent dispatch by independent orchestrators becomes impossible rather than merely discouraged, and the unguarded discovery-mode submission path is covered by construction rather than by a second bespoke check.
- Multi-operator use of one workspace becomes coherent: filing and inspecting work stays open, so several people can work different features while exactly one drives execution.
- **Cost:** a third exclusivity concept joins declared ownership and run leases. Orbit will hold reservations, leases, and claims — all TTL-bounded exclusive holds at different granularities. Without deliberate vocabulary discipline these will be confused for one another.
- **Cost:** a dead holder blocks dispatch until the TTL elapses. Force-release is the necessary escape hatch and also the thing that weakens the guarantee, since a habitual force-release makes the claim advisory in practice.
- **Cost:** the claim token is state the holder must keep. A client that loses it must wait out the TTL or force, even though it is the legitimate holder.
- **Cost:** two coordination dimensions now share one table. The discriminator is compiler-enforced at every query site and claim rows carry no files, but a future reader of `task_reservations` must still learn that the table holds two things.
- **Cost:** this contradicts the resident-orchestrator design, which chose one-active-epic plus non-overlapping routine fires plus a host pin specifically to avoid introducing a lease or assignee subsystem. That decision is revised, not left standing in contradiction.
- Declared ownership stays what it is. The claim answers "which operator is driving this workspace right now", not "which machine holds the canonical checkout", and the two must not be collapsed.