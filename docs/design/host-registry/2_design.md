---
title: Host Registry — Design
owner: claude
last_updated: 2026-07-20
status: Accepted
feature: host-registry
doc_role: design
type: design
summary: Target mechanisms for host identity, the main-host registry, the coordination-plane/workspace-ownership split, pull-based execution placement, per-record data placement, and the routine ownership revision.
tags: [host-registry, multi-host, dispatch, routines, data-placement]
paths: ["crates/orbit-remote/**", "crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-mcp/**", "crates/orbit-common/**"]
related_features: [host-registry, mcp-bridge, routines, remote-access, mcp-session-context]
related_artifacts: [ORB-00424, ORB-10247, ORB-10248, ORB-10249, ORB-10255, ORB-10257, ORB-10258, ORB-10267, ORB-10268, ORB-10269, ORB-10271, ORB-10272, ORB-10302, ORB-10319, ORB-10330, ORB-10332, ADR-0200, ADR-0205, ADR-0208, ADR-0226, ADR-0227, ADR-0228, ADR-0229, ADR-0230, ADR-0231, ADR-0232, ADR-0235, ADR-0240]
---

# Host Registry — Design

This doc specifies the **target** design. Host identity, the logical workspace
catalog, registry core/projections, operator administration, sanitized discovery,
and the satellite-cache format landed through C4. E1's strict hub trust document
and fixed checkoutless hub MCP endpoint, E2's bounded verified spoke link, and E3's
private registration plus first remote coordination slice have landed
[ORB-10268, ORB-10269, ORB-10271]. [ORB-10319] then consolidated those coupled
pieces into the vertical `orbit-remote` feature crate described by [ADR-0240],
superseding the earlier horizontal boundary in [ADR-0235]. [ORB-10272] adds the
dormant hub-global ADR/learning sequence substrate inside that boundary; public
knowledge creation remains on the standalone compatibility path until the F3
cutover. Run
placement, polling, and later phases remain pending. The folder is Accepted. It
covers host identity, the registry, the
coordination-plane/workspace-ownership split, execution placement (including the
hub→satellite protocol), the per-record data-placement split, and the revision to
routine sweep ownership. It leaves client→hub transport to the
[MCP bridge](../mcp-bridge/2_design.md) ([ORB-00424]) and everything speculative to
[3_vision.md](./3_vision.md).

## 1. Host Identity (`host.toml`)

`~/.orbit/host.toml` remains the one genuinely host-local datum ([ADR-0205]), widened
from a scheduling pin to a versioned machine identity ([ORB-10247]):

```toml
# ~/.orbit/host.toml
schema_version = 1           # on-disk version; a higher value fails closed
machine_id     = "hm_9f2c81d4"   # generated once at init; never edited or reused
host_id        = "dk-mac"        # human name; unique across the registry
mode           = "standalone"    # standalone | hub | spoke
```

**Initialization moves to `orbit init`.** Global seeding prompts for a host name
(default: OS hostname; refuses to prompt when stdin is not a TTY), asks for the
operating `mode` (default `standalone`), and generates `machine_id` if absent.
Non-interactive callers pass `--host-name <name>` (and optionally `--host-mode`); a
fresh host initialized non-interactively without `--host-name` fails closed rather
than defaulting silently. `orbit routine init` continues to work but no longer owns
the file — it reads the existing identity (limiting its own mutation to clock
installation) and errors if none exists, replacing today's silent hostname fallback.

**Migration is once and idempotent.** A legacy `host.toml` carrying only `host_id`
(the routines-v1 scheduling pin) is upgraded in place on `orbit init`: `host_id` is
preserved, `mode` defaults to `standalone`, and `machine_id` is generated once and
never regenerated. A repeated init preserves the `machine_id` and writes nothing.
The write is atomic (staged rename), so rollback always leaves the last valid
identity readable and a partially overwritten file is impossible.

**Loading is strict.** After migration, identity resolution (routine `hosts:`
matching, the sweep, `routine status`) fails closed with an actionable error on an
absent, malformed, incomplete, blank, or future-schema file — it never falls back to
the OS hostname, and a newer `schema_version` fails without rewriting the file.
`machine_id` must remain in the generated `hm_` namespace with a path- and
transport-free ASCII suffix; values shaped like hostnames, SSH destinations, paths,
or URI targets are rejected before they can enter a registry or workspace role.

**Resolution rule: names are for humans, `machine_id` is what the system stores.**
Human-authored text — routine `hosts:` pins, the task `host` selector, CLI arguments
— uses `host_id` and resolves through the registry *at the moment of binding or
dispatch*. Everything the system persists after resolution — workspace ownership
bindings, run placement snapshots, leases, audit provenance — stores `machine_id`
(with the name alongside for display). A rename therefore cannot silently redirect
an existing binding; it can only strand *unresolved* human-authored text, which pin
validation catches (§2, §6).

The current implementation lives in `orbit_remote::host_identity`; CLI and routine
callers import the owning feature crate directly. [ORB-10302] first extracted this
domain into `orbit-registry`; [ORB-10319] renamed and widened that crate without
changing the identity contract ([ADR-0235], [ADR-0240]).

## 2. The Registry

The registry is the main host's inventory of known machines — a `hosts` table in the
main host's global store (`~/.orbit/orbit.db`), not a per-machine file. A registry
nobody can enumerate isn't a registry.

Per entry: `machine_id` (key), `host_id` (globally reserved across active and retired entries),
`labels` (free-form: providers installed such as `claude`/`codex`, OS), a
**workspace presence map** (below), `status` (`active` | `retired`),
`registered_at`, `last_seen`.

- **Registration.** `orbit host register` run on the main host directly, or from a
  satellite over the SSH-carried MCP path. Satellite registration happens only
  after trusted local MCP config pins the hub's out-of-band-copied `machine_id`;
  see [mcp-bridge/2_design.md §1](../mcp-bridge/2_design.md). Registration is
  idempotent on `machine_id`; a name collision with a different `machine_id` is an
  error. The spoke path uses the connector-private
  `orbit/private/register-spoke/v1` request after MCP negotiation. It is absent
  from canonical schemas and `tools/list`; an unknown caller may invoke only this
  typed request. Results name the last committed registry/presence/profile/snapshot
  stage, never claim distributed rollback, and refresh the local sanitized cache
  only after a definitive complete response [ORB-10271].
- **Workspace presence map.** Each entry carries `{workspace_id → {root,
  last_verified}}`: where that host has each workspace checked out, reported from
  the host's own local workspace registry at registration and refreshed on every
  runner poll (§4). This is a load-bearing field, not an optional label — placement
  validation (§4) requires the target host to advertise a checkout of the task's
  workspace. The leased run carries the stable workspace ID; the satellite resolves
  its execution path through its own local registry. The hub's absolute paths never
  cross into satellite execution.
- **Enumeration.** The hub-local human `orbit host list` command renders the
  single-transaction `RegistrySnapshotV1`, including retired identities; a spoke fails
  closed instead of presenting its local shadow database as the hub inventory. The
  `orbit.host.list` MCP discovery tool was removed in [ORB-10332]; the parallel
  `orbit.workspace.list` and `orbit.crew.list` MCP discovery tools remain and return
  entries with labels, presence, and freshness so the orchestrator can right-size
  placement and crew.
- **Liveness.** `last_seen` is updated by registration and by every runner poll
  (§4) — the poll *is* the heartbeat, at the same minute cadence as the sweep. It is
  **not** derived from existing audit rows: the audit `host` column records the
  hostname of the process executing the call, which for SSH-carried MCP is the hub
  itself. Instead, MCP session metadata (see the `mcp-session-context`
  feature) carries separate caller/process `machine_id` and display `host_id`
  values, stamped onto audit rows as additive provenance. The legacy audit `host`
  remains the executing-process hostname; neither it nor the new process fields are
  overwritten with caller identity. External MCP JSON cannot supply these trusted
  values. The local adapter establishes them before preflight, while an authenticated
  managed envelope wins over caller claims ([ORB-10228]).
- **Rename.** `orbit host rename` updates `host_id` on the entry (keyed by
  `machine_id`) and the local `host.toml`, and keeps the old name as a **tombstone
  alias** mapping to the same `machine_id`: stale human-authored references resolve
  with a warning instead of silently failing or being hijacked. A tombstoned or
  retired name can never be claimed by a *different* `machine_id`. The rename
  command reports where the old name still appears in committed text; it does not
  rewrite other repos. A machine-global sibling lock spans current-name preflight,
  the staged local write, the SQLite rename, and any post-error outcome probe, so
  concurrent local rename commands cannot each report success with opposing final
  identities. Invalid names fail before the local file changes; a reported SQLite
  commit error is classified by reopening the registry rather than being claimed as
  an unconditional rollback.
- **Retire.** Retired hosts stay in the registry so old provenance, pins, and
  bindings keep resolving; dispatch and pins targeting a retired host fail
  validation with the retirement visible in the error.

**Concrete registry-core schema ([ORB-10255]).** Store migration v5 adds two
hub-global tables without changing any existing row or standalone path:

- `hosts` is keyed by immutable `machine_id` and carries the current globally
  reserved `host_id`, canonical label-set JSON, `active|retired` status,
  `registered_at`, `updated_at`, optional `retired_at`, and optional
  `last_seen_at`. Initial registration is an explicit observation, so it seeds
  `last_seen_at`; a compatible repeated registration is a true no-op and does not
  move any timestamp. Later poll/heartbeat updates remain C2 work and must be
  explicit — audit traffic is never a liveness input.
- `host_aliases` is keyed by the historical human name and carries only its stable
  `machine_id`, creation timestamp, and warning text. Database triggers make alias
  rows update/delete-proof and enforce current-name/alias disjointness across the
  two tables; a retired row's current name remains unique as well.

Typed store operations take an immediate transaction before collision preflight.
Registration is idempotent only for the same active `machine_id`, `host_id`, and
label set; it cannot rename, relabel, or reactivate. Rename changes the current name
and inserts the old one as a tombstone in one transaction, so chained renames keep
the full history. Retirement changes lifecycle state without deleting either table.
Resolution returns an explicit active, alias-with-warning, retired, unknown, or
fail-closed collision projection. `HostRegistryService` binds these operations to
B1's `HostIdentity` declaration. CLI/MCP administration and local `host.toml`
rename coordination landed in C3 ([ORB-10267]): `orbit host register/list/rename/retire`,
the `orbit.host.list` (since removed in [ORB-10332]) and `orbit.workspace.list` discovery
tools, one path-free `RegistrySnapshotV1` projection, the atomic satellite registry cache,
and the hub-global `registry_revision` (store schema v8).

`HostRegistryService` now lives in `orbit_remote::host_registry`, backed by
`RemoteStore` in `orbit_remote::persistence`. Remote owns the registry SQL, row
codecs, revision advancement, snapshot transaction, and a feature-schema migration
ledger entry. `orbit-store` supplies only generic SQLite connection, transaction,
and namespaced feature-migration machinery; it does not import Remote. Remote v1
adopts the immutable global v5/v6/v8 registry tables in place and refuses an unknown
future Remote schema instead of copying rows or creating a second database
([ORB-10319], [ADR-0240]).

Remote feature migration v2 adds the dormant hub knowledge-ID tables in that same
config-resolved database ([ORB-10272]). Opening `RemoteStore` applies only the
schema: it does not inspect a checkout, reconcile a workspace, advance either
sequence, or activate hub authority. This preserves standalone creation and makes
activation an explicit, restart-safe transition rather than a side effect of
opening the hub.

**Boundary with `~/.orbit/mcp.toml`.** The registry is server-side *inventory*;
`mcp.toml` is the client's trust policy for its one hub route. They stay separate:
registry state must never grant a capability, and a repo checkout must never mutate
the registry — the same non-elevation rule the MCP bridge imposes on plugin config.
The E1 implementation [ORB-10268] loads that trust file only from the machine-global
root and verifies the hub server's store stamp against `host.toml` before listing or
dispatch, so neither repository configuration nor a shadow coordination database
can redirect the authority boundary.

## 3. Coordination Plane and Workspace Ownership

The topology is a **star**: spokes only ever initiate connections to the hub, the
hub never connects out (it queues; satellites poll, §4), and no machine ever needs a
route to another machine. Adding a host is register + poll — zero pairwise setup.
Within that star, two concepts that earlier drafts conflated as "authority" are kept
separate:

**The coordination plane is fixed at the main host.** Tasks, review threads,
artifacts, frictions, the run queue, the registry, and *all* global ID allocation
live on the hub for every workspace, regardless of which machine owns the repo. This
is a v1 invariant, not per-workspace configuration: one coordination writer, one
place to triage, one MCP target for the orchestrator.

**Workspace ownership is per-machine.** Each workspace has exactly one **owner**:
the machine holding the canonical checkout, serving as the default execution host
(§4), and solely authoring that workspace's knowledge records (§5). Ownership is a
declared binding, never an inference — a workspace checked out on three machines
still has exactly one named owner. It is recorded twice, once on each side, both
locally readable:

- On the **hub**: `workspace_ownership` stores the stable workspace ID and owner
  `machine_id`. Operator input may use a human `host_id`, but `orbit workspace link`
  resolves it once; the current display name is joined from the host registry only
  when rendering discovery and is not a second ownership key.
- On each **machine**: the logical workspace record mirrors the owner `machine_id`,
  while its local checkout records `owner` or `replica`. A replica repeats only the
  same stable owner `machine_id`; no display name or transport target is persisted.
  `orbit workspace init --role ...` establishes a new checkout and `orbit workspace
  role` reasserts a compatible local declaration. The trusted hub `machine_id` →
  SSH-target mapping is machine-level state in `~/.orbit/mcp.toml`; workspace role
  never grants or redirects access to its owner.

**Concrete local registry schema ([ORB-10248]).** `~/.orbit/workspaces.json` is
versioned independently of `host.toml`. Schema v1 has two collections:

- `workspaces: Vec<Workspace>` is the path-free logical catalog. Each record keeps
  stable ID/name, owner `machine_id` (optional only for pre-identity standalone
  installs), Git/workflow metadata, status, and timestamps. A hub may therefore
  retain a workspace it cannot check out without fabricating a path.
- `checkouts: Vec<WorkspaceCheckout>` is machine-local. Each binding names the
  `workspace_id`, `repo_root`, `orbit_dir`, path overrides, and `role`. `owner`
  carries no replica owner field; `replica` must carry `owner_machine_id`, which
  must equal the logical record's stable owner.

ID/name lookup reads only `workspaces`. Path lookup uses longest-prefix matching
across `WorkspaceCheckout.repo_root` and that checkout's overrides; a checkoutless
logical workspace can never resolve to a path. Existing list output renders `-`
for such a record rather than inventing a root.

Legacy unversioned registries migrate in one atomic write in standalone mode: every
legacy workspace becomes one logical record plus one local owner checkout, while
IDs, names, Git/workflow fields, status, timestamps, and valid overrides are
retained. A second load is byte-stable. Pre-host-identity input is the standalone
compatibility case; an absent local role canonicalizes to `owner`. Hub/spoke loads,
including unversioned input without an explicit role, require stable owner identity
and reject missing/unknown roles, owner/replica contradictions, and replicas without
an owner, naming the workspace ID. Malformed/future schemas are read-only failures,
and a failed staged write leaves the prior file readable.

New multi-host checkouts declare both sides before the first registry write:
`orbit workspace init` defaults to an explicit local-owner binding, while `--role
replica --owner <machine_id>` writes the remote logical owner and local replica
mirror together. Persisted hub/spoke input with an owner checkout but no logical
owner is rejected; loading never backfills ownership from local machine identity.

The catalog/checkout implementation now lives in
`orbit_remote::workspace_registry`; CLI, dashboard, and execution callers use that
feature API directly. There is no `orbit-core` compatibility re-export or duplicate
implementation.

**Concrete hub coordination projections ([ORB-10257]).** Additive store migration
v6 creates three path-separated projections:

- `workspace_ownership` binds each stable logical `workspace_id` to exactly one
  active, known `owner_machine_id`. Binding validates the logical workspace through
  `HostRegistryService`; the B2 `owner_machine_id` is checked only as the local
  mirror. Rebinding is not inferred or silently performed.
- `host_workspace_presence` is private and keyed by
  `(machine_id, workspace_id)`. An authenticated publication atomically replaces
  that host's complete declared map and explicitly stamps host `last_seen_at`.
  The reported absolute root is the sole path-bearing hub-link exception: it is
  retained only in this private placement projection and is absent from sanitized
  reads, profiles, audit, leases, routing frames, and remote execution.
- `workspace_execution_profiles` stores one owner payload plus hub-owned
  `generation` and `received_at`. The frozen `ExecutionProfileV1` is exactly
  identity, owner `observed_at`, `config_digest`, normalized effective crews, and
  `{mode, base_branch, ship_closure_digest}`. Identical semantic publication
  refreshes both timestamps without advancing generation; a semantic change
  advances it atomically. Owner authentication, generation CAS, active ownership,
  already-stale/future observations, and observations older than the stored one
  all fail before overwrite. Freshness is calculated only from hub `received_at`.

`config_digest` hashes domain-separated canonical compact JSON of the normalized
crew/config and effective mode/base branch. `ship_closure_digest` separately hashes
the execution-selected, fully materialized four-job ship closure, its reachable
named and recovery activities, resolved backends, and versioned static ship
contract. Neither digest contains identities, clocks, paths, raw config/assets or
environment values. Publication rejects execution-affecting catalog/backend
environment overrides and unknown provider/backend values rather than publishing
an ambiguous projection. These projections are typed service/store foundations;
administration, dispatch gating, run snapshots, leases, and connectors remain later
units.

`orbit_remote::build_execution_profile_v1` owns profile construction and combines
Remote workspace authority with Core's transport-neutral execution-environment
snapshot and ship-closure digest. Publication, ownership, presence, freshness, and
sanitized registry administration remain in the same Remote feature boundary.
Core knows neither the registry nor Remote.

### 3.1 Vertical feature boundary

Host registry and MCP bridge are one feature with one evolution boundary:

```text
orbit-cli / orbit-dashboard
  └── orbit-remote
        ├── identity, workspace catalog, cache, profiles, routines
        ├── persistence (registry SQL over the shared orbit.db)
        └── MCP composition, broker, hub, link, registration
              ├── orbit-core   (transport-independent runtime/coordination executor)
              ├── orbit-store  (generic SQLite and feature-migration kernel)
              ├── orbit-tools  (generic builtin tool definitions)
              ├── orbit-mcp    (generic RMCP framing and raw client)
              ├── orbit-graph  (local-derived graph query engine; extraction folded in as a module, ORB-10357)
              └── orbit-common (shared DTOs)
```

The database remains the config-resolved shared `orbit.db`; vertical ownership does
not mean a `remote.db`. `orbit-remote` owns the feature's tables and transactions
through `RemoteStore`, while Store owns connection lifecycle and the generic ledger.
Likewise, Remote owns registry-aware schema composition and placement policy while
MCP owns only protocol mechanics. Core's checkout-independent
`HubCoordinationExecutor` stays transport-neutral and is invoked by Remote's hub and
broker rather than importing Remote. These acyclic seams let a remote change evolve
inside one crate without turning the neutral kernels into feature modules
([ORB-10319], [ADR-0240]).

Enforcement reads only local data, so it works offline; what fails offline is the
MCP write itself, loudly. Two local rules (§5 for the record types they guard):

1. **Machine-level:** a machine that is not the hub mutates coordination records
   only via MCP to the hub. Decided from the machine's own config, set at init.
2. **Workspace-level (replica mode):** a machine that is not a workspace's owner
   does not author that workspace's knowledge records locally. Decided from the
   machine's own workspace entry.

For validation that *does* need registry data on a satellite (routine pin checks,
§6), the satellite keeps a **registry cache**: a snapshot refreshed on every
successful poll or register. Cache semantics are explicit: enforcement never reads
it (enforcement is local-only, above); validation reads it and degrades to
warning-only when the cache is absent or stale. The cache stores one sanitized hub
snapshot plus a machine-local receipt timestamp. Conflict comparison uses stable hub
identity, revision, records, and authoritative timestamps; read-time-derived
freshness/age views are not canonical mutations. Equal-revision/equal-canonical
refresh preserves the cached snapshot and renews only the local receipt, while a
different hub, lower revision, or stable payload conflict fails without replacing
prior bytes. A missing or invalid hub identity is never a usable cache: first refresh
fails before writing, and a persisted unpinned snapshot reloads as malformed without
being rewritten. Scheduling keeps working offline.

The codec, canonical comparison, freshness, and crash-classification behavior live
in `orbit_remote::registry_cache`. Shared `RegistryCacheV1` and
`RegistrySnapshotV1` DTOs stay in `orbit-common`; `RemoteStore` is the sole producer
of the transactional sanitized snapshot over Store's generic connection kernel.

[ORB-10271] wires this cache contract to spoke registration. Identity comes only
from validated local `host.toml`; presence and owner profiles come only from the
typed local registry/runtime builders. An `outcome_unknown`, a partial hub result,
or a local cache-write failure preserves the prior valid cache and is never retried
as registration. A cache failure after complete hub success reports the confirmed
hub commit and the local repair requirement separately.

Neither role is ever selected per-task: coordination has one writer by construction,
and two owners for one workspace is the split-brain the system already rejected
([ADR-0200]).

**Concrete task coordination schema ([ORB-10249]).** The hub task registry's
`workspace_bindings` table is a path-free logical record: `workspace_id`, slug,
optional repository fingerprint, and timestamps. Machine-local paths live only in
the optional one-to-one `workspace_checkout_bindings` table (`workspace_id`,
`repo_root`, `workspace_path`, `orbit_dir`, timestamps). Allocator, canonical task
bundle, workspace index, tag, and relation rows reference the logical
`workspace_id`; none requires a checkout row. Canonical bundles remain in the
hub's coordination tree, so a checkoutless workspace can create, read, update, and
schedule tasks without a fabricated repository path. Checkout-local projections
resolve the optional checkout binding first and fail before filesystem mutation,
naming the workspace when absent.

Task IDs remain globally unique. Every ORB-valued dependency or typed-relation
target resolves through the whole coordination registry, while task list/index
queries remain workspace-scoped. The global status projection supplies dependency
readiness across workspaces without opening either checkout. Missing ORB targets
are rejected before task allocation or bundle/index mutation with both target ID
and source workspace in the error. Schema v4 migrates each legacy path-coupled row
into one logical record plus one checkout binding without changing task IDs,
canonical bundle paths, payloads, relations, workspace associations, or allocator
state; repeated open/reindex is idempotent.

## 4. Execution Placement

Execution placement — which machine runs the agent — is per-task, orthogonal to
ownership, and **pull-based**. [ORB-00424] carries MCP from a client *to* the hub;
nothing in it sends work back out. This section is that missing half, and it
deliberately reuses the runner model rather than inventing a push channel: the hub
is a mailbox, not a relay — it queues placed runs, and satellites collect their own.
The hub never opens a connection to a satellite.

- **Shipping is opt-in per task; manual execution is first-class.** Creating a task
  files a coordination record — it never implies dispatch. A task sits in
  `proposed`/`backlog` until the orchestrator ships it *or a human claims it*. A
  claimed task is excluded from ship triage and gets no run, lease, or placement
  snapshot: the human works in any local checkout, sends coordination writes
  (status, comments, resolution) over MCP from wherever they are, and lands code
  through the repo's normal gate — PR into `agent-main` for gated repos, direct
  commit otherwise. Resolution cites the PR or commit instead of run artifacts.
  Knowledge records on the manual path still allocate their global ID from the hub
  first (§5); the narrative file then rides the same PR — the store record and the
  committed file are already decoupled, which is what makes this work without a
  reservation protocol. Everything below concerns *shipped* runs.
- **Preference.** Tasks gain an optional `host` selector — a preference, not a
  binding. It is human-authored text (`host_id`), validated at triage/dispatch
  against the registry exactly as `crew` is: unknown or retired names fail with the
  valid names in the error; a name whose presence map lacks the task's workspace
  fails with what that host does advertise. Unset means the workspace's **owner** —
  work defaults to the machine holding the canonical checkout, not to the hub.
- **Placement snapshot.** At dispatch, `workflow.ship` resolves the preference and
  stamps the run with `placement.requested = {host_id as written, resolved
  machine_id}`. When a host takes the run, `placement.actual = machine_id` is
  recorded at lease time. Both are immutable on the run record: retries and rescues
  create new runs that *re-resolve* the task-level preference, and audit history
  always shows what was asked versus what happened.
- **Hub-placed runs** (runs for hub-owned workspaces, or explicitly placed there)
  execute exactly as today: in-process dispatch on the main host. No new machinery
  on the common path.
- **Satellite-placed runs** enter a `placed` state. Each satellite runs a
  **runner poll** on the same minute-cadence clock unit that drives the sweep: it
  calls `orbit.run.lease` over the SSH-carried MCP path, identifying itself by
  `machine_id`. The hub atomically leases at most the runs placed on that machine
  (TTL'd, the same shape as the existing `task_reservations` table). The satellite
  executes locally in its own checkout (path from its own workspace registry — the
  same data it advertised into the presence map), performs all coordination writes
  back over MCP against the hub, and reports terminal state via `orbit.run.report`.
- **Lease expiry** returns the run to `placed` — visible to the shepherd/rescue
  flow, never silently re-placed onto a different host. Re-placement is an
  orchestrator decision, consistent with no-auto-discovery dispatch.
- **Capability.** Runner traffic gets its own capability set, `runner` — lease,
  report, presence refresh, and nothing else. A satellite's standing credential is
  narrower than `operator`; it cannot ship work, enumerate hosts, or author records
  beyond its own run reports.
- **Non-owner execution** is permitted when the presence map advertises a checkout:
  coordination writes go over MCP and code lands on a git branch either way. Its
  limit is knowledge authoring (§5) — a run on a non-owner machine cannot file
  learnings for a workspace the hub doesn't own, so runs expected to produce
  knowledge belong on the owner (which the default already selects).

## 5. Data Placement

Per-record placement rules, chosen to dissolve sync rather than implement it:

| Record type | Writes | Reads | Why |
|---|---|---|---|
| Tasks, review threads, artifacts | MCP to the main host (in-process there) | MCP | Coordination plane: single ID allocator, lifecycle churn, one writer, no merge |
| Frictions | MCP to the main host (in-process there) | MCP | Coordination lifecycle (raise → triage → resolve), same shape as tasks |
| Learnings, ADRs | Owner-only, into the owner's checkout; global ID allocated by the main host | Owner: local. Elsewhere: MCP for hub-owned workspaces; explicit git-replica reads after `git pull` + reindex | One writer per workspace; git carries a readable replica outward without becoming the live transaction path |
| Code graph, docs index | Local | Local | Derived from the local checkout, per-branch, rebuildable |
| Routine scheduler state | Local | Local | Host-local by design ([ADR-0208]); cursors and pauses never sync |

Notes:

- **F1 installs the hub-global allocator but does not cut callers over.** Remote
  feature migration v2 creates independent monotonic `adr` and `learning`
  sequences, a reconciliation projection, an immutable allocation ledger, and a
  dormant/active authority marker in the hub's shared `orbit.db` [ORB-10272]. The
  existing standalone/worktree allocator and all current ADR/learning create paths
  remain unchanged. F3 alone activates public issuance and replaces those callers;
  a standalone host cannot enter hub authority merely by opening the database.
- **Activation validates the complete hub-local inventory before mutation.** The
  hub inventories every registered workspace from locally available migration
  sources: ADR and learning files in every valid lifecycle state plus every legacy
  allocation row, including reserved, unfinalized, and abandoned rows. A missing
  source fails precondition with the workspace named; the hub never contacts or
  proxies to its owner. The full inventory is validated before a sequence,
  reconciliation row, ledger row, or audit row changes. Every cross-workspace
  duplicate `(kind, id)` is reported with all conflicting workspace/source
  evidence and is never renumbered.
- **Seeding is forward-only and restart-safe.** Under one hub write lock, a final
  reseed advances each sequence above its independently computed global maximum,
  records the reconciled workspace/source digest, and activates authority in one
  durable transition. Repeating or reopening that transition is idempotent and
  can never decrease a sequence. A workspace registered after activation is
  knowledge-ineligible until the same complete local reconciliation succeeds
  under the allocator lock; registration cannot make an unscanned workspace
  eligible.
- **Allocation is one atomic hub transaction.** A successful allocation advances
  exactly one kind's sequence, appends an immutable row keyed by `mcp_call_id`, and
  writes one canonical SQLite audit carrying the trusted ORB-10271 caller/process
  provenance. Exact replay is idempotent only when the full stored request identity
  matches; reusing a correlation ID for another request fails without advancement.
  Internal lookup exists by correlation and by `(workspace, kind, id)`. Invalid
  kind/workspace/correlation, ineligible workspace, and overflow likewise leave
  both sequences unchanged.
- **Allocated IDs are final gaps, not reservations.** There is no release,
  abandon, expiry, reuse, or remote-finalize API. Owner finalization may fail after
  allocation and leave a valid unused ID. That gap is deliberate and does not
  grant the hub a route to a spoke owner.

- **Knowledge is one-writer per workspace — the owner.** The owner authors the file
  in its own checkout and commits there; the global ID comes from the hub in a
  single allocation call first (in-process when the owner *is* the hub). Allocation
  and finalization are seconds apart on one machine with one writer, so there is
  deliberately **no** reservation/finalization protocol — no reservation expiry,
  orphaned IDs, or finalize/pull races to design. The hub's ID sequence stays
  single-authority for every record type.
- **Non-owner knowledge authoring is unsupported in v1** — for agents. A machine
  that doesn't own the workspace doesn't author its learnings/ADRs through the CLI
  or MCP; anything actionable becomes a task addressed to the owner. The default
  placement rule (§4) makes this rare: runs execute where knowledge can be filed.
  The one escape hatch is deliberate: replica mode guards the *store/CLI* surface,
  not git — a human on the manual path (§4) may carry a knowledge file in a PR,
  with the global ID allocated from the hub first and the repo gate as the
  arbiter.
- **Canonical MCP reads don't span owners.** The hub serves current-state knowledge
  reads for workspaces it owns; in the star topology it does not proxy content
  reads to other owners. A git-replicated learning/ADR is an explicit local/offline
  read path and may be stale — the MCP broker never silently substitutes it when
  the canonical source is unreachable; see
  [mcp-bridge/2_design.md §6](../mcp-bridge/2_design.md).
- **Placement rules are enforced, not advisory.** The two local rules in §3
  (machine-level for coordination records, replica mode for knowledge records)
  reject the write at the CLI, decided from local data. The task export/import
  renumbering machinery (`orbit-store/src/task_migration/`) stops being the
  multi-machine story and reverts to what it is: a migration tool.
- **Replica knowledge reads have a catch.** The learning envelope *index* lives in
  each machine's local `orbit.db`; a non-owner reading learnings from its checkout
  needs a reindex-from-files pass after pull. Until that exists, replica learning
  reads go over MCP (hub-owned workspaces) — correctness before convenience.

## 6. Routine Ownership Revision

Current contract ([routines/2_design.md](../routines/2_design.md)): the sweep loads
definitions from every registered workspace with `[routines] role = "source"`,
filters to routines whose `hosts:` pin contains this `host_id`, and dispatches due
targets. That mechanic stays. What changes is the semantics around committed
definitions:

**Rule: the host pinned in a git-committed routine definition is the host in charge
of that routine.** Consequences:

1. **Unpinned committed routines fail closed.** A committed definition with no
   `hosts:` pin is a load-time lint error — never "any host." An unpinned routine
   checked out on N source-role machines is N independent schedules; refusing to
   load is the only default that can't double-fire. (The existing per-host name
   collision error already catches the two-checkouts-on-one-machine case.)
2. **Scope by location, not git status.** Personal routines live in
   `.orbit/routines/local/`, gitignored by convention. They are implicitly pinned to
   the local host; a `hosts:` pin naming another machine there is an error. The
   sweep never shells out to `git check-ignore` — the directory is the contract.
3. **Registry-validated pins, cache-degraded.** `orbit routine list` and the sweep
   resolve pins against the registry — through the local registry cache (§3) on
   satellites. Unknown names, retired hosts, and tombstoned aliases are flagged;
   with poll-driven `last_seen`, so is a routine whose owning host has gone quiet.
   When the cache is absent or stale, validation degrades to warning-only and
   scheduling proceeds: a routine must fire offline, and its own host pin is
   matchable against local `host.toml` without any registry at all.
4. **`role = "source"` becomes a discovery hint, not a trust boundary.** The pin is
   the guard, and it is reviewable in git like everything else.
5. **Reassignment semantics.** Scheduler cursors are host-local ([ADR-0208]) and do
   not migrate. Editing a pin in git moves the routine to a host with no cursor
   history: the new host's first sweep records a baseline and schedules from now —
   no backfill, and `catch_up_once` applies within a host's own history only.

## 7. Concerns & Honest Limitations

- **A disconnected satellite cannot do task or friction work.** Coordination
  writes fail loudly offline. This is the intended trade — fail loudly rather than
  fork state — but it makes the main host's availability a hard dependency for all
  dispatch.
- **The main host is a single point of failure — and now, so is each owner.** The
  coordination plane (registry, ID allocators, run queue) lives on `dk1`; hub down
  means all dispatch stalls. Owner down means that workspace's default execution
  and all its knowledge authoring stall, even though its tasks can still be filed.
  This design makes both dependencies explicit; it mitigates neither.
- **Placement latency is bounded by poll cadence.** A satellite-placed run waits up
  to a minute before pickup, plus lease semantics on failure. Acceptable for this
  system's task shapes; wrong for anything interactive.
- **Satellites hold standing credentials.** Every polling satellite has a
  persistent SSH identity that reaches the hub with `runner` capability. The set is
  deliberately narrow, but each registered host is now part of the hub's attack
  surface, and revocation (retire + key removal) is a manual runbook.
- **`last_seen` measures polling, not health.** A host whose poller runs but whose
  provider binaries are broken looks alive. Freshness gates dispatch eligibility;
  it does not verify the run will succeed.
- **Presence maps trust the satellite.** A stale or wrong advertised root fails at
  execution time on the satellite, not at validation time. `last_verified` bounds
  but does not eliminate this.
- **Renames leave stale text in other repos.** Tombstone aliases keep old names
  resolving-with-warning, but committed pins on old names are debt the rename
  command can report, not fix — and the alias table is append-only forever.
- **Cross-machine knowledge doesn't flow live.** A non-owner cannot author a
  workspace's knowledge at all, and cannot read its *current* state unless the hub
  owns it — only the git replica, which lags. This is the cost of one writer per
  workspace and a hub that never proxies; it stays cheap only while the hub owns
  nearly everything.
- **Enforcement is per-surface plumbing.** Both local rules (§3) need the refusal
  path wired into every CLI surface that can mutate a guarded record type; a missed
  surface is a silent local fork until noticed. Needs a test that walks the
  registered tool surface.

## Task References

- [ORB-00424] — proposed the local/remote Orbit MCP unification (SSH-carried stdio,
  capability sets) that carries client→hub traffic; this design adds the
  hub→satellite half as a pull-based runner protocol.
- [ORB-10247] — implemented the versioned `HostIdentity` (§1): `schema_version` /
  `machine_id` / `host_id` / `mode`, `orbit init` ownership, legacy migration, and
  strict fail-closed loading (Phase 1 / Unit B1 under ORB-10246).
- [ORB-10248] — implemented the versioned path-free workspace catalog and
  machine-local owner/replica checkout bindings (§3; Phase 1 / Unit B2).
- [ORB-10255] — implemented the append-only v5 host/alias schema and typed C1
  registry core: compatible idempotent registration, collision refusal, permanent
  rename chains, retirement, active enumeration, and fail-closed name resolution.
- [ORB-10257] — implemented additive v6 singular workspace ownership, private
  host-keyed presence replacement/freshness, and owner-authenticated execution
  profile CAS with canonical config and fully materialized ship-closure digests.
- [ORB-10258] — implemented origin-aware routine loading (§6 items 1–2; Unit R1 under
  ORB-10246): committed definitions fail closed without a non-empty host pin,
  `.orbit/routines/local/` definitions are implicit to the loading host and reject
  remote pins, and cross-origin name collisions fail deterministically.
- [ORB-10270] — implemented Unit R2: routine list/show/sweep now validate committed pins
  through the hub snapshot or classified spoke cache before scheduler mutation; expose
  stable cache/host diagnostics; preserve offline exact-local eligibility; and prove the
  A-to-B reassignment boundary (A unchanged, B baseline, next natural slot only).
- [ORB-10302] — repurposed `orbit-registry` as the host/workspace domain crate,
  moved its domain tests with the implementations, retained runtime profile/ship
  hashing in `orbit-core`, and preserved store ownership of persistence
  ([ADR-0235]).
- [ORB-10319] — widens and renames that extraction to the vertical `orbit-remote`
  feature: registry persistence, profile/cache/routine composition, MCP contract,
  broker, hub, link, and registration share one crate, while Store, MCP, Core,
  Tools, and Common remain neutral acyclic dependencies ([ADR-0240]).
- [ORB-10269] — implemented the fixed SSH command, contract/digest negotiation,
  one bounded peer per scalar capability, trusted remote metadata, and the
  pre-handoff `hub_unavailable` / post-handoff `outcome_unknown` no-replay split.
- [ORB-10271] — implemented connector-private spoke registration, current active
  caller validation on every ordinary hub call, staged projection/snapshot results,
  definitive-success cache refresh, operator-only friction list/show, and the
  hermetic two-root coordination/provenance canary.
- [ORB-10272] — adds Remote feature migration v2 and the dormant hub-global ADR and
  learning sequence service: complete validated hub-local reconciliation,
  forward-only activation, replay-safe immutable correlation ledger, atomic audit,
  and explicit late-workspace ineligibility without changing standalone creation
  or activating the F3 cutover.
- [ORB-10332] — removed the `orbit.host.list` MCP discovery tool as unused; the
  `orbit host list` CLI command and the `orbit.workspace.list` / `orbit.crew.list`
  MCP discovery tools remain.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
