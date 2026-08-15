---
title: Host Registry — Design
owner: claude
last_updated: 2026-08-12
last_validated: 2026-08-10
status: Draft
feature: host-registry
doc_role: design
type: design
summary: Mechanisms for host identity and machine task prefix, per-workspace ownership held in the local registry, the workspace claim gating workflow dispatch, workspace-scoped data placement, and the routine ownership revision; fleet registration and execution placement are deferred to v2.
tags: [host-registry, multi-host, ownership, routines, data-placement]
paths: ["crates/orbit-remote/**", "crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-mcp/**", "crates/orbit-common/**"]
related_features: [host-registry, mcp-bridge, routines, remote-access, mcp-session-context, resident-orchestrator]
related_artifacts: [ORB-00424, ORB-10247, ORB-10248, ORB-10249, ORB-10255, ORB-10257, ORB-10258, ORB-10267, ORB-10268, ORB-10269, ORB-10271, ORB-10272, ORB-10302, ORB-10319, ORB-10330, ORB-10332, ORB-10709, ORB-10725, ORB-10730]
---

# Host Registry — Design

This doc specifies the **v1 target** design: every machine is its own coordination
host for the workspaces it owns, a machine-scoped task prefix supplies global ID
uniqueness by partition, and the machine-local workspace registry is the source of
truth for ownership. It covers host identity and prefix, the local registry, the
ownership rules and their enforcement, the workspace claim gating dispatch,
workspace-scoped data placement, and the routine ownership revision. Client→owner
transport belongs to the [MCP bridge](../mcp-bridge/2_design.md) ([ORB-00424]);
deferred work is in [3_vision.md](./3_vision.md).

> **Status: Draft — structural rewrite in flight.** This revision supersedes the
> singular-hub model ([Singular coordination hub, workspace owner, and per-run placement](./4_decisions.md#singular-coordination-hub-workspace-owner-and-per-run-placement), [Owner-authored knowledge with hub-global IDs and explicit replicas](./4_decisions.md#owner-authored-knowledge-with-hub-global-ids-and-explicit-replicas), [Pull-based leases with immutable placement and explicit recovery](./4_decisions.md#pull-based-leases-with-immutable-placement-and-explicit-recovery)). Substantial machinery
> for that model has already shipped: the hub-side `hosts`/`host_aliases` tables
> and operator administration [ORB-10255, ORB-10267], the strict hub trust
> document and checkoutless endpoint [ORB-10268], bounded spoke link and private
> self-registration [ORB-10269, ORB-10271], and the dormant hub-global
> ADR/learning sequence substrate [ORB-10272]. Sections below mark each piece
> **survives**, **deferred to v2** (built, dormant, retained), or **retire**
> (built, contradictory, to be removed). The vertical `orbit-remote` boundary
> [ORB-10319] / [Consolidate remote host and MCP behavior in the vertical orbit-remote crate](./4_decisions.md#consolidate-remote-host-and-mcp-behavior-in-the-vertical-orbit-remote-crate) is unaffected throughout.

## 1. Host Identity (`host.toml`)

`~/.orbit/host.toml` remains the one genuinely host-local datum ([Routine discovery through workspace registry](../routines/4_decisions.md#routine-discovery-via-the-workspace-registry-and-a-versioned-routines-rolesource-config-key)), widened
from a scheduling pin to a versioned machine identity ([ORB-10247]):

```toml
# ~/.orbit/host.toml
schema_version = 2           # on-disk version; a higher value fails closed
machine_id     = "hm_9f2c81d4"   # generated once at init; never edited or reused
host_id        = "dk-mac"        # human name
task_prefix    = "DE"            # namespace for every task id this machine mints
```

There is no `mode` field ([Every machine is its own coordination host](./4_decisions.md#every-machine-is-its-own-coordination-host)). A machine's role is not a declaration; it is
derived per workspace from the local registry (§2). A machine that owns every
workspace it holds is what earlier drafts called `standalone`, and it needs no
configuration to say so.

**`task_prefix` is the one genuinely new field** ([Machine-scoped task-id prefix instead of a global allocator](./4_decisions.md#machine-scoped-task-id-prefix-instead-of-a-global-allocator)). Every task this
machine mints is `<task_prefix>-NNNNN` against this machine's own monotonic
sequence. Uniqueness across machines follows from the prefixes differing — a
human-scale, once-per-machine choice, not a coordinated allocation. Rules:

- **Chosen at global init, immutable after.** Task IDs leak into commit messages,
  branch names, and committed knowledge records; a prefix change would strand all
  of them. Changing it is a migration, not a setting.
- **Validated against the reserved namespaces.** `ORB`, the legacy `ADR` prefix,
  `L`, and `F` are refused: they are current or historical artifact-reference
  namespaces, and minting tasks with the legacy decision prefix would break
  `related_artifacts` parsing (see
  `crates/orbit-core/src/command/docs/artifact_ref.rs`). Also refused: anything
  that is not 2–5 uppercase ASCII letters.
- **Existing installs keep `ORB`.** The v1→v2 identity migration writes
  `task_prefix = "ORB"` for any host that already has a task sequence, so no
  existing ID changes and no existing citation breaks.
- **Collision is possible and deliberately unguarded in v1.** Two machines can
  pick the same prefix; nothing detects it until they meet. This is tolerable
  precisely because the failure is benign — see §3 on why prefix partition makes
  divergent ownership recoverable by union — and because v2 registration can
  reject a duplicate at join time.

Parsing must therefore become prefix- and width-agnostic. `ORB_TASK_ID_PREFIX` /
`ORB_TASK_ID_WIDTH` in `crates/orbit-common/src/types/task_artifacts.rs` are the
declaration site; `is_valid_orb_task_id`, `parse_orb_task_number`, the
`artifact_ref.rs` predicates, and the byte-scanner in
`crates/orbit-core/src/command/skill.rs` all hardcode the constant today. A
prefix-agnostic scanner is weaker against prose than a fixed one, so the scanner
matches only prefixes known to the local registry rather than any
`[A-Z]{2,5}-[0-9]+` shape.

**Initialization moves to `orbit init`.** Global seeding prompts for a host name
(default: OS hostname; refuses to prompt when stdin is not a TTY) and a task
prefix, and generates `machine_id` if absent. Non-interactive callers pass
`--host-name <name>` and `--task-prefix <prefix>`; a fresh host initialized
non-interactively without either fails closed rather than defaulting silently.
`orbit routine init` continues to work but no longer owns the file — it reads the
existing identity (limiting its own mutation to clock installation) and errors if
none exists, replacing today's silent hostname fallback.

**Migration is once and idempotent.** A legacy `host.toml` carrying only `host_id`
(the routines-v1 scheduling pin) is upgraded in place on `orbit init`: `host_id` is
preserved, `machine_id` is generated once and never regenerated, and `task_prefix`
is seeded as described above. A `mode` key from the superseded model is dropped on
read and not rewritten. A repeated init preserves both `machine_id` and
`task_prefix` and writes nothing. The write is atomic (staged rename), so rollback
always leaves the last valid identity readable and a partially overwritten file is
impossible.

**Loading is strict.** After migration, identity resolution (routine `hosts:`
matching, the sweep, `routine status`) fails closed with an actionable error on an
absent, malformed, incomplete, blank, or future-schema file — it never falls back to
the OS hostname, and a newer `schema_version` fails without rewriting the file.
`machine_id` must remain in the generated `hm_` namespace with a path- and
transport-free ASCII suffix; values shaped like hostnames, SSH destinations, paths,
or URI targets are rejected before they can enter a registry or workspace role.

**Resolution rule: names are for humans, `machine_id` is what the system stores.**
Human-authored text — routine `hosts:` pins, CLI arguments — uses `host_id` and
resolves *at the moment of binding*. Everything the system persists after
resolution — workspace ownership bindings, audit provenance — stores `machine_id`
(with the name alongside for display). A rename therefore cannot silently redirect
an existing binding; it can only strand *unresolved* human-authored text, which pin
validation catches (§2, §6). In v1 resolution consults the machine-local registry
rather than a fleet inventory, so a name belonging to another machine resolves only
if this machine has recorded it as some workspace's owner.

The current implementation lives in `orbit_remote::host_identity`; CLI and routine
callers import the owning feature crate directly. [ORB-10302] first extracted this
domain into `orbit-registry`; [ORB-10319] renamed and widened that crate without
changing the identity contract ([Make orbit-registry the singular host/workspace registry domain crate](./4_decisions.md#make-orbit-registry-the-singular-hostworkspace-registry-domain-crate), [Consolidate remote host and MCP behavior in the vertical orbit-remote crate](./4_decisions.md#consolidate-remote-host-and-mcp-behavior-in-the-vertical-orbit-remote-crate)).

## 2. The Local Workspace Registry

**v1 has no fleet inventory.** Each machine's `workspaces.json` is the source of
truth for what that machine owns, and no machine holds a catalog of the others
([Every machine is its own coordination host](./4_decisions.md#every-machine-is-its-own-coordination-host), [Defer fleet registration and execution placement to v2](./4_decisions.md#defer-fleet-registration-and-execution-placement-to-v2)). This is the smallest thing that supports the actual
requirement — keep some projects local, keep task IDs unique — and it is what makes
the whole model shippable without a registration protocol.

Per workspace, the local registry records the existing [ORB-10248] shape: the
stable logical `workspace_id` and name, the local checkout `root`, an
`owner_machine_id`, and a local `role` of `owner` or `replica`. The invariants the
existing implementation already enforces carry over unchanged — a machine cannot
assign itself `owner` over a workspace that names a different owner, a `replica`
role requires an explicit non-local `owner_machine_id`, and contradictory owner
declarations between the workspace record and the checkout binding are refused.

- **Ownership is declared, self-asserted, and unverified.** `orbit workspace
  adopt` marks a workspace owned by this machine; `orbit workspace link
  --owner <machine_id>` records a replica checkout of someone else's. Nothing
  arbitrates competing claims in v1 — two machines can both believe they own a
  workspace, and neither will find out until they meet.
- **That failure is benign, and this is why v1 can skip registration.** Because
  every machine mints task IDs under its own prefix (§1), two machines that
  diverge on ownership produce *disjoint* task sets. Repair is picking a winner
  and taking the union; there is no ID to renumber and no merge to resolve. The
  prefix decision and the no-registration decision hold each other up — dropping
  either one alone would be unsafe.
- **Enumeration is local and role-filtered.** `orbit workspace list` shows the
  workspaces this machine owns. Replica checkouts are listed only under
  `--all`, visibly marked with their owner. `orbit host list`, the fleet
  inventory command, has nothing to enumerate in v1 and is withdrawn rather than
  left returning a single row.
- **Replica checkouts stay in the registry.** Hiding them from `list` is a display
  rule, not a deletion: the entry is what lets `resolve.rs` map a cwd to a
  workspace by longest-prefix match, and the entire local-derived tier — code
  graph, docs index, docs search — needs no coordination authority at all and must
  keep working inside a checkout you don't own. Removing the entry would break all
  of that to express something the `role` field already expresses.
- **Rename.** `orbit host rename` updates `host_id` in the local `host.toml` and in
  any local workspace record that names this machine as owner. Without a fleet
  registry there is no cross-machine alias table in v1; a machine renamed while
  another machine holds replica checkouts pointing at its old *name* is a v2
  concern, mitigated meanwhile by the fact that bindings persist `machine_id`, not
  the name. `task_prefix` is never touched by a rename (§1).

**Liveness has no v1 meaning.** `last_seen` was maintained by the runner poll,
which is deferred with the rest of execution placement (§4). Nothing in v1 needs
to know whether another machine is up: you find out when you call it. The audit
provenance work is independent and survives — MCP session metadata carries
caller/process `machine_id` and display `host_id`, stamped onto audit rows as
additive provenance, and is never derived from the audit `host` column (which
records the executing process's hostname). External MCP JSON cannot supply these
trusted values; the local adapter establishes them before preflight, and an
authenticated managed envelope wins over caller claims ([ORB-10228]).

### 2.1 The shipped hub registry — deferred, not deleted

The hub-side inventory is built and works. It is **deferred to v2**: retained in
the tree, unreachable from any v1 code path, and reactivated when workspace
registration is designed ([3_vision.md §1](./3_vision.md)). The detail below
describes what exists so a reader does not mistake dormant tables for dead code.
Built and dormant: `orbit host register/list/rename/retire` and the private
`orbit/private/register-spoke/v1` self-registration path [ORB-10267, ORB-10271];
the per-entry **workspace presence map** `{workspace_id → {root, last_verified}}`
that placement validation consumed; the single-transaction path-free
`RegistrySnapshotV1` projection and its atomic satellite cache; **tombstone
aliases**, which keep a renamed host's old name resolving to the same
`machine_id` so stale human-authored text warns instead of silently failing or
being hijacked; and **retirement**, which keeps old provenance and bindings
resolving while failing validation for anything targeting the retired host. The
tombstone and retirement semantics are the parts worth preserving verbatim — they
are the answers to questions v2 will ask again.

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
([ORB-10319], [Consolidate remote host and MCP behavior in the vertical orbit-remote crate](./4_decisions.md#consolidate-remote-host-and-mcp-behavior-in-the-vertical-orbit-remote-crate)).

**Remote feature migration v2 — retire.** [ORB-10272] added dormant hub-global
`adr` and `learning` sequences, a reconciliation projection, an immutable
allocation ledger, and a dormant/active authority marker. Unlike the registry
tables, these do not survive as deferred work: [Workspace-scoped knowledge keys, no global knowledge IDs](./4_decisions.md#workspace-scoped-knowledge-keys-no-global-knowledge-ids) removes global knowledge
IDs entirely in favour of `(workspace_id, artifact_key)`, so the substrate is not
merely unused but contradictory. It was never activated — public creation stayed on
the compatibility path — so removal drops schema that never issued an ID. Parking
unused code is cheap; parking code that encodes a superseded model is not.

Removal cannot simply delete the v2 registry entry. `Store::apply_feature_migrations`
validates a database's recorded ledger against the shipped registry position by
position and refuses a changed migration name or a version it does not know, so a
database that recorded `dormant_hub_knowledge_sequences` at v2 must still find that
name at v2. [ORB-10725] therefore keeps the slot and empties it — a fresh database
never creates the tables — and adds v3 `drop_dormant_hub_knowledge_sequences`,
which drops every object `IF EXISTS`. One migration serves both populations: it
removes the substrate from a database that applied the original v2 and passes
through a database that never had it, so the two converge on the same schema.

**Boundary with `~/.orbit/mcp.toml`.** `mcp.toml` is the client's trust policy for
the routes it will initiate. In v1 it may name **more than one** route, since a
machine may hold replica checkouts of workspaces owned by several other machines
(§3); each entry pins one target `machine_id` copied out of band. The
non-elevation rule is unchanged and load-bearing: a repo checkout must never
mutate routing or grant a capability, which is why ownership is recorded in the
machine-local registry and never in committed workspace config. The E1
implementation [ORB-10268] loads that trust file only from the machine-global root
and verifies the server's store stamp against `host.toml` before use, so neither
repository configuration nor a shadow coordination database can redirect the
authority boundary. That check generalizes to per-route verification unchanged.

## 3. Coordination Plane and Workspace Ownership

**The coordination plane follows ownership.** A workspace's tasks, review threads,
and artifacts live in the store of the machine that owns it. Every machine is
therefore a coordination host — for its own workspaces and no others. There is no
fleet-wide coordination target, no machine-level role, and no configuration
required to keep a project local: a workspace nobody else owns simply coordinates
where it already is ([Every machine is its own coordination host](./4_decisions.md#every-machine-is-its-own-coordination-host)).

The invariant that matters is preserved exactly: **one coordination writer per
workspace.** The superseded model achieved that by making one writer for
*everything*, which is a strictly stronger claim than the invariant needs and the
reason local-only projects had to route through a machine they had no business
touching.

The topology is no longer a single star. A machine initiates connections to the
owners of workspaces it holds replicas of, and nothing initiates back — the
direction that mattered (client → owner) is the one the SSH-carried MCP path
already provides. There is no queue, no mailbox, and no pairwise setup beyond a
route entry in `mcp.toml`.

**Workspace ownership is per-machine and declared.** Each workspace has exactly one
**owner**: the machine holding the canonical checkout, coordinating its tasks, and
authoring its knowledge records (§5). Ownership is a declared binding, never an
inference — a workspace checked out on three machines still has exactly one named
owner. In v1 it is recorded in exactly one place, the owner-and-replica sides of
each machine's own local registry:

- The logical workspace record carries the owner `machine_id`; the local checkout
  records a `role` of `owner` or `replica`. A replica repeats only the same stable
  owner `machine_id` — no display name and no transport target is persisted.
  `orbit workspace init --role ...` establishes a new checkout and `orbit workspace
  role` reasserts a compatible local declaration.
- The `machine_id` → SSH-target mapping stays machine-level state in
  `~/.orbit/mcp.toml`. Workspace role never grants or redirects access to an owner:
  knowing who owns a workspace and being able to reach them are deliberately
  separate, so a checkout can never widen its own access.

**Coordination writes in a non-owned checkout fail closed.** Hiding another
machine's workspaces from `list` is not sufficient — a read-side filter would still
let `orbit task add` mint a local record for a workspace this machine does not own,
splitting the task set across two stores. The write is refused at the same shared
chokepoint that enforces the workspace claim (§3.2), so every surface inherits it,
and the error names the owning `machine_id` and the configured route to reach it if
one exists. Automatically forwarding the call to the owner is deliberately *not*
v1: an implicit cross-machine write is exactly the kind of thing that should be
typed out once by a human before it becomes a default.

Reads in a non-owned checkout are unaffected for the local-derived tier (§5) and
return empty for coordination records — `orbit task list` in someone else's
workspace shows nothing, because this machine holds nothing.

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

Legacy unversioned registries migrate in one atomic write: every legacy workspace
becomes one logical record plus one local owner checkout, while IDs, names,
Git/workflow fields, status, timestamps, and valid overrides are retained. A second
load is byte-stable. An absent local role canonicalizes to `owner`, which is the
correct default under this model — a machine that never declared anything owns what
it holds. Any record carrying an explicit role must satisfy the stable-owner
identity rules: missing or unknown roles, owner/replica contradictions, and
replicas without an owner are rejected, naming the workspace ID. Malformed and
future schemas are read-only failures, and a failed staged write leaves the prior
file readable.

Note that this schema needs no change to support v1. [ORB-10248] already split the
path-free logical catalog from machine-local checkout bindings and already carries
`owner_machine_id` plus the `owner`/`replica` role. What this revision removes is
the layer that was *above* it — the machine-level mode and the fleet coordination
target — not the substrate, which turns out to have been the right shape.

New checkouts declare their side before the first registry write: `orbit workspace
init` defaults to an explicit local-owner binding, while `--role replica --owner
<machine_id>` writes the remote logical owner and local replica mirror together.
An owner checkout with no logical owner is rejected; loading never backfills
ownership from local machine identity.

The catalog/checkout implementation lives in `orbit_remote::workspace_registry`;
CLI, dashboard, and execution callers use that feature API directly. There is no
`orbit-core` compatibility re-export or duplicate implementation.

**Hub coordination projections ([ORB-10257]) — deferred to v2.** Store migration v6
created three hub-side projections that presuppose a fleet inventory:
`workspace_ownership` (a second, hub-side ownership key, redundant now that the
local registry is authoritative), `host_workspace_presence` (the presence map,
which only placement consumed), and `workspace_execution_profiles` (one owner
payload plus hub-owned `generation`/`received_at`, so a hub could right-size crew
for a workspace it does not hold).

All three are dormant in v1 and none is read on a v1 path. They are retained
because v2 registration will want exactly this shape, and because
`workspace_execution_profiles` carries a genuinely reusable piece: the frozen
`ExecutionProfileV1` and its two digests. `config_digest` hashes domain-separated
canonical compact JSON of the normalized crew/config and effective mode/base
branch; `ship_closure_digest` separately hashes the execution-selected, fully
materialized four-job ship closure, its reachable named and recovery activities,
resolved backends, and versioned static ship contract. Neither digest contains
identities, clocks, paths, raw config/assets, or environment values. That
construction is transport-independent and stays useful whether or not a profile is
ever published to another machine.

`orbit_remote::build_execution_profile_v1` owns profile construction and combines
Remote workspace authority with Core's transport-neutral execution-environment
snapshot and ship-closure digest. Core knows neither the registry nor Remote.

### 3.1 Vertical feature boundary

Host registry and MCP bridge are one feature with one evolution boundary:

```text
orbit-cli / orbit-dashboard
  └── orbit-remote
        ├── identity, workspace catalog, cache, profiles, routines
        ├── persistence (registry SQL over the shared orbit.db)
        └── MCP composition, broker, owner link (registration dormant, §2.1)
              ├── orbit-core   (transport-independent runtime/coordination executor)
              ├── orbit-store  (generic SQLite and feature-migration kernel)
              ├── orbit-tools  (generic builtin tool definitions)
              ├── orbit-mcp    (generic RMCP framing and raw client)
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
([ORB-10319], [Consolidate remote host and MCP behavior in the vertical orbit-remote crate](./4_decisions.md#consolidate-remote-host-and-mcp-behavior-in-the-vertical-orbit-remote-crate)).

**Enforcement collapses to one rule, and it is entirely local.** The superseded
model needed two — a machine-level rule for coordination records and a
workspace-level rule for knowledge records — because the two record classes had
different authorities. With coordination following ownership, both classes have the
same one:

> A machine that is not a workspace's declared owner does not write that
> workspace's coordination or knowledge records. Decided from the machine's own
> workspace entry, with no remote call and no cached fleet state.

Enforcement therefore works offline by construction; what fails offline is a
deliberate cross-machine call, loudly. Dropping the machine-level rule also removes
the last consumer of the `mode` field, which is why §1 can delete it outright
rather than deprecate it.

The **registry cache** — a sanitized hub snapshot refreshed on every successful
poll or register, used to validate routine pins on a satellite — is deferred with
registration (§2.1). Its purpose was validating *someone else's* host names, and
v1 has no such names to validate: a routine pin either matches this machine's
`host.toml` or names a machine in this machine's own workspace records (§6). The
implementation in `orbit_remote::registry_cache` and the shared `RegistryCacheV1` /
`RegistrySnapshotV1` DTOs in `orbit-common` are retained for v2.

Ownership is never selected per-task: coordination has one writer by construction,
and two owners for one workspace is the split-brain the system already rejected
([Live remote/multi-workspace dashboard viewing supersedes the git-sync task registry](../remote-access/4_decisions.md#live-remotemulti-workspace-dashboard-viewing-supersedes-the-git-sync-task-registry)).

**Concrete task coordination schema ([ORB-10249]).** The task registry's
`workspace_bindings` table is a path-free logical record: `workspace_id`, slug,
optional repository fingerprint, and timestamps. Machine-local paths live only in
the optional one-to-one `workspace_checkout_bindings` table (`workspace_id`,
`repo_root`, `workspace_path`, `orbit_dir`, timestamps). Allocator, canonical task
bundle, workspace index, tag, and relation rows reference the logical
`workspace_id`; none requires a checkout row. Canonical bundles remain in the
coordinating machine's own tree, so a checkoutless workspace can create, read,
update, and schedule tasks without a fabricated repository path. Checkout-local
projections resolve the optional checkout binding first and fail before filesystem
mutation, naming the workspace when absent. All of this is unchanged by the
revision — it was never hub-specific, only described that way.

**Task IDs stay globally unique, by partition rather than by allocator.** Each
machine's sequence is monotonic within its own prefix (§1), so an ID is unique
across every machine without any of them agreeing on anything. Two consequences
follow, and both are real costs:

- **Relation targets resolve only within one coordinating machine.** Dependency
  and typed-relation targets resolve through that machine's coordination registry;
  list/index queries remain workspace-scoped, and the status projection supplies
  dependency readiness across the workspaces that machine owns. A target belonging
  to a workspace owned elsewhere cannot be validated. Today the store *rejects*
  unresolvable targets, which would make a legitimate cross-machine reference
  impossible to record. v1 therefore distinguishes them: a target carrying a
  **foreign prefix** is stored as an unvalidated reference and rendered with a
  "not verifiable here" marker, while a target under a **local prefix** that does
  not resolve stays a hard error naming both target ID and source workspace. The
  weaker guarantee applies only where the stronger one is unobtainable.
- **Cross-machine chronology is gone.** A single sequence made `ORB-10601` visibly
  later than `ORB-10248`; per-machine sequences do not compare. `created_at` is the
  ordering key, and no reader should infer sequence from ID.

Schema v4 migration is unaffected: each legacy path-coupled row becomes one logical
record plus one checkout binding without changing task IDs, canonical bundle paths,
payloads, relations, workspace associations, or allocator state, and repeated
open/reindex is idempotent. Existing IDs keep the `ORB` prefix (§1), so no migration
touches an ID.

### 3.2 Workspace claim

Ownership (§3) answers *which machine* holds the canonical checkout. It does not
answer *which operator is driving the workspace right now*, and that question now
has more than one possible answer: an off-box orchestrator over the owned tunnel
([mcp-bridge/2_design.md §5.3](../mcp-bridge/2_design.md)), a local operator
broker, and a session over SSH can all reach one workspace concurrently. Two
operator sessions on the same machine are indistinguishable to the ownership
model.

The existing guards do not cover this. The duplicate-dispatch guard is keyed on
task id and scans a bounded window of recent runs, so a stale non-terminal run
outside that window is invisible — and auto/backlog-discovery submissions carry no
task ids at all, so two discovery ship runs in one workspace both proceed. Task
reservations (§5) are file-scoped and arbitrate between *workers*, not between
orchestrators.

A **workspace claim** is an exclusive, TTL-bounded hold taken by one operator
([Gate workflow dispatch on an exclusive TTL'd workspace claim](./4_decisions.md#gate-workflow-dispatch-on-an-exclusive-ttld-workspace-claim)). Its scope is deliberately narrow:

- **It gates workflow dispatch only** — the governed `orbit.workflow.*` operations.
  Filing tasks, reading, updating, searching, and authoring knowledge are
  unaffected and stay concurrent. Several people working different features in one
  workspace is the intended behaviour; only the decision of *what starts* is
  serialized.
- **Enforcement is at the shared run-submission path**, not at a protocol adapter,
  so every surface inherits it — CLI, HTTP, MCP, and remote command execution
  alike. This is the same placement that makes the duplicate-dispatch guard
  surface-independent, and it is why a caller holding shell cannot route around
  the claim: the CLI reaches the same chokepoint.
- **Acquisition mints a claim token** returned to the holder and presented on
  subsequent workflow calls. Machine and session identity are recorded for
  diagnostics but are not load-bearing: MCP session identity is minted per
  connection and does not survive a reconnect, so keying the claim on it would
  orphan the workspace every time a client reconnects.
- **Contention rejects**, carrying the current holder and the expiry instant.
  Never a silent queue, never a silent steal.
- **TTL-bounded with lazy expiry** evaluated on each check, plus an explicit,
  audited force-release for a holder that has gone away.

Claim scope is a **distinct dimension** from file reservations. Expressing it as a
whole-workspace file selector would also block the worker reservations it is meant
to leave alone, inverting the intent.

**An unclaimed workspace gates nothing** ([ORB-10709]). The claim arbitrates
between operators who want one; it is not a precondition a dispatch must satisfy
in an uncontended workspace. Concretely, as shipped:

| State | `orbit.workflow.ship` / `run.resume` |
| --- | --- |
| Unclaimed | proceeds |
| Claimed, caller presents the holder's token | proceeds |
| Claimed, caller presents nothing or a stale token | refused with holder + expiry |

The token is presented as `claim_token` on the tool, CLI, and HTTP surfaces, or
through `ORBIT_WORKSPACE_CLAIM_TOKEN` for an operator shell. Acquire, release,
force-release, and status are the `orbit.workspace.claim.*` tools, registered the
same way as `orbit.task.locks.*` — operator-reachable, absent from the agent MCP
surface. Storage reuses `task_reservations` under a `scope` discriminator; see
[Gate workflow dispatch on an exclusive TTL'd workspace claim](./4_decisions.md#gate-workflow-dispatch-on-an-exclusive-ttld-workspace-claim)
for why, and for the rejected parallel-table alternative.

The claim does not replace or weaken declared ownership. Ownership remains a
declared machine binding that selects default execution and gates knowledge
authoring; the claim is a runtime hold on dispatch authority. Collapsing the two
would reintroduce exactly the split-brain [Live remote/multi-workspace dashboard viewing supersedes the git-sync task registry](../remote-access/4_decisions.md#live-remotemulti-workspace-dashboard-viewing-supersedes-the-git-sync-task-registry) rejected.

> This revises the reasoning in
> [resident-orchestrator/2_design.md §3](../resident-orchestrator/2_design.md),
> which chose one-active-epic plus `overlap: forbid` plus a host pin specifically
> to avoid a lease or assignee subsystem. Those constraints bound *automated*
> routine fires; they do not arbitrate between interactive operator sessions,
> which is the case that forced this decision.

## 4. Cross-Machine Access

**Execution runs where the workspace is owned. There is no placement in v1**
([Defer fleet registration and execution placement to v2](./4_decisions.md#defer-fleet-registration-and-execution-placement-to-v2)). A run for a workspace executes on that workspace's owner, in-process,
exactly as a single-machine install does today. No `host` selector, no `placed`
state, no run queue, no leases, no runner poll, no `runner` capability. The entire
hub→satellite half of the earlier design is withdrawn.

This is a scope decision, not a discovery that placement is unworkable. Placement
was designed to answer "run this task on that machine," and nothing in the current
system needs that answered: the workspace's owner is already the machine with the
checkout, the provider credentials, and the knowledge-authoring rights. What is
actually needed across machines is much smaller.

**The v1 cross-machine surface is task coordination, and nothing else.** From any
machine, an operator or agent can create, read, and update tasks in a workspace
owned by another machine by pointing a client at that machine's MCP endpoint over
the existing SSH route ([ORB-00424],
[mcp-bridge/2_design.md §5](../mcp-bridge/2_design.md)). This is the client→owner
direction that already works; no new transport, no inbound listener, and no
credential a satellite must hold standing.

Consequences worth stating plainly:

- **Direction matters and is asymmetric.** A machine can reach an owner it has a
  route to. An owner cannot reach out to it. Any workflow that assumes the reverse
  — dispatching work *to* a laptop from a server — is v2.
- **Shipping is opt-in per task; manual execution is first-class.** Creating a task
  files a coordination record — it never implies dispatch. A task sits in
  `proposed`/`backlog` until the orchestrator ships it *or a human claims it*. A
  claimed task gets no run: the human works in a local checkout, sends coordination
  writes to the owner, and lands code through the repo's normal gate — PR into
  `agent-main` for gated repos, direct commit otherwise. Resolution cites the PR or
  commit instead of run artifacts. Knowledge records on this path need no
  allocation call at all now (§5) — the file is written in the owner's checkout and
  rides the same PR.
- **A workspace whose owner is unreachable is not workable remotely.** Its tasks
  cannot be filed or read from elsewhere until the owner is back. The superseded
  model concentrated that dependency on one machine for the whole fleet; this one
  distributes it per workspace, which is better for blast radius and worse for
  predictability — you now have several machines that can each take part of the
  system offline.

## 5. Data Placement

Per-record placement rules, chosen to dissolve sync rather than implement it:

| Record type | Writes | Reads | Why |
|---|---|---|---|
| Tasks, review threads, artifacts | Owner-only, in the owner's store (MCP to the owner from elsewhere) | Owner: local. Elsewhere: MCP to the owner | Coordination lifecycle, one writer per workspace, no merge |
| Frictions | Owner-only, keyed `(workspace_id, friction_key)` | Same as tasks | Coordination lifecycle (raise → triage → resolve), same shape as tasks |
| Learnings, ADRs | Owner-only, into the owner's checkout, keyed `(workspace_id, artifact_key)` | Owner: local. Elsewhere: git after `pull` + reindex | One writer per workspace; git carries the record outward with no live transaction path and no allocator |
| Code graph, docs index | Local | Local | Derived from the local checkout, per-branch, rebuildable — works in a non-owned checkout |
| Routine scheduler state | Local | Local | Host-local by design ([Routine definitions are git-shared; scheduler state is host-local and never synced](../routines/4_decisions.md#routine-definitions-are-git-shared-scheduler-state-is-host-local-and-never-synced)); cursors and pauses never sync |

**There is no global ID allocator for any record type** ([Workspace-scoped knowledge keys, no global knowledge IDs](./4_decisions.md#workspace-scoped-knowledge-keys-no-global-knowledge-ids)). Tasks are
unique by machine prefix (§1); frictions, learnings, and ADRs are unique by
`(workspace_id, artifact_key)` and make no claim to be unique outside their
workspace. This mostly ratifies what was already true — friction and run IDs have
always been per-workspace — and extends it to ADRs and learnings, which were the
only records that needed a fleet authority.

Notes:

- **The dormant hub-global allocator is removed, not parked.** [ORB-10272]'s `adr`
  and `learning` sequences, reconciliation projection, allocation ledger, and
  authority marker were built to issue IDs that no longer exist. Because public
  issuance was never activated, nothing has to be renumbered and no ID was ever
  allocated from them. See §2.1 for why this is deleted while the registry tables
  are kept.
- **Knowledge is one-writer per workspace — the owner** — and now needs no protocol
  to be so. The owner writes the file in its own checkout and commits. With no
  allocation call there is no reservation, no expiry, no orphaned ID, and no
  finalize/pull race: the two-step allocate-then-finalize sequence that the
  superseded design worked hard to keep atomic simply does not occur.
- **Cross-workspace decision titles are not identities.** A decision titled
  "orbit-web reloads the workspace registry per request" in one workspace and a
  same-titled decision in another are different records. Any merged,
  cross-workspace search result **must** carry the `workspace` field; a bare title
  from such a result is not addressable. This is the
  existing friction-ID footgun — an ID taken from a merged search and fed to a
  workspace-scoped write hits the wrong record or none — generalized to more record
  types, so fixing the projection is a precondition of this change, not a follow-up.
- **Decision references are repo-local and never cited across repos**; see
  [CONVENTIONS.md §4c](../CONVENTIONS.md#4c-format-and-links). Decision reasoning
  lives in git-committed entries in each feature's
  `4_decisions.md`, which removes the last consumer of hub-allocated knowledge IDs.
- **Non-owner knowledge authoring stays unsupported** — for agents. A machine that
  doesn't own the workspace doesn't author its learnings or ADRs through the CLI or
  MCP; anything actionable becomes a task addressed to the owner. Since execution
  now always happens on the owner (§4), this case barely arises. The escape hatch is
  unchanged and deliberate: ownership guards the *store/CLI* surface, not git — a
  human may carry a knowledge file in a PR, with the repo gate as arbiter.
- **Reads do not span owners.** A machine serves current-state knowledge reads for
  workspaces it owns and never proxies content reads to another owner. A
  git-carried learning or ADR read from a replica checkout is an explicit
  local/offline path and may be stale; the MCP broker never silently substitutes it
  when the canonical source is unreachable. See
  [mcp-bridge/2_design.md §6](../mcp-bridge/2_design.md).
- **Enforcement is local and non-advisory.** The single ownership rule in §3 rejects
  the write at the shared chokepoint, decided from local data. The task
  export/import renumbering machinery (`orbit-store/src/task_migration/`) stops
  being the multi-machine story and reverts to what it is: a migration tool — and
  gains a genuine use, since moving a workspace between owners is now a supported
  operation that copies rows without touching an ID (§3).
- **Replica knowledge reads still have a catch.** The learning envelope *index*
  lives in each machine's local `orbit.db`; a non-owner reading learnings from its
  checkout needs a reindex-from-files pass after pull. Until that exists, replica
  learning reads require the owner over MCP — correctness before convenience.

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
3. **Locally-validated pins.** `orbit routine list` and the sweep resolve pins
   against local data only: this machine's `host.toml` and the owner names recorded
   in its own workspace registry (§2). A pin naming this machine matches and fires;
   a pin naming a machine this host knows of is reported as *belongs elsewhere*; a
   pin naming nothing recognizable is flagged as unresolvable. This is strictly
   weaker than the superseded registry validation — without a fleet inventory
   there is no way to distinguish "typo" from "a machine I have not heard of," and
   no `last_seen` with which to notice that a routine's owning host has gone quiet.
   Both losses are accepted: the pin's *own-host* case, which is the one that
   decides whether anything fires, is decidable offline from `host.toml` alone, and
   that was always the load-bearing half.
4. **`role = "source"` becomes a discovery hint, not a trust boundary.** The pin is
   the guard, and it is reviewable in git like everything else.
5. **Reassignment semantics.** Scheduler cursors are host-local ([Routine definitions are git-shared; scheduler state is host-local and never synced](../routines/4_decisions.md#routine-definitions-are-git-shared-scheduler-state-is-host-local-and-never-synced)) and do
   not migrate. Editing a pin in git moves the routine to a host with no cursor
   history: the new host's first sweep records a baseline and schedules from now —
   no backfill, and `catch_up_once` applies within a host's own history only.

## 7. Concerns & Honest Limitations

- **Prefix uniqueness has no authority, and the failure is silent.** Two machines
  can pick the same `task_prefix` and nothing notices until their records meet in a
  merged view. The mitigation is that the collision is *recoverable* rather than
  prevented — but "recoverable" assumes someone notices. There is no lint, no
  warning, and no join-time check in v1.
- **Ownership is self-asserted.** `workspaces.json` is unverified, so two machines
  can both claim a workspace and each will behave correctly in isolation. Divergence
  is discovered by a human, not by the system, and the longer it runs the more
  there is to reconcile — even though reconciliation itself is a union.
- **Every owner is now a single point of failure for its own workspaces.** The
  superseded model concentrated this on one machine, which was worse for blast
  radius and better for predictability: one thing to check, one thing to keep up.
  Now a workspace is remotely unusable whenever its owner is down, and which
  workspaces those are depends on a per-machine file. This design makes the
  dependency explicit; it mitigates it in neither direction.
- **`orbit workspace list` hiding replicas will confuse someone.** A checkout is
  present, `cd` works, docs search works, and the workspace is absent from `list`
  and refuses `task add`. That is three different answers to "is this workspace
  here," all correct, and the error message is the only thing that explains it.
- **Cross-machine task relations are only partly validated.** A foreign-prefix
  target is stored unverified (§3), so a typo in a cross-machine reference is
  indistinguishable from a legitimate one until someone follows it. The alternative
  — refusing them outright — was worse, but this is a real hole where the
  superseded single-registry model had none.
- **Per-machine sequences destroy cross-machine chronology.** Nothing can be
  inferred from comparing two IDs any more. Readers who have internalized "higher
  ORB number means later" will be wrong and will not be told.
- **Three names for adjacent concepts remain.** File reservations (§5) and the
  workspace claim (§3.2) are both "a temporary exclusive hold with an expiry" at
  different granularities. Retiring run leases removes the third, but the remaining
  two still need vocabulary discipline to stay distinct.
- **A dead claim holder blocks dispatch until its TTL elapses.** Force-release is
  the necessary escape hatch and simultaneously the thing that weakens the
  guarantee: a force-release that becomes habitual makes the claim advisory in
  practice, which is the failure mode of every lock that ships with an override.
- **The claim token is client-held state.** A holder that loses it must wait out
  the TTL or force-release, despite being the legitimate holder. Binding the claim
  to session identity instead would be worse — MCP session identity is minted per
  connection, so every reconnect would orphan the workspace.
- **Renames are weaker than they were.** Without the fleet registry there are no
  tombstone aliases in v1, so a renamed machine strands any human-authored text
  naming its old name. Persisted bindings store `machine_id` and are unaffected,
  which bounds the damage to committed pins and prose.
- **Cross-machine knowledge doesn't flow live.** A non-owner cannot author a
  workspace's knowledge and cannot read its current state at all — only what git
  carries, which lags. Previously the hub could at least serve reads for everything
  it owned, which was nearly everything.
- **Enforcement is per-surface plumbing.** The ownership rule (§3) needs the refusal
  path wired into every surface that can mutate a guarded record type; a missed
  surface is a silent local fork until noticed. Needs a test that walks the
  registered tool surface.
- **The revision strands shipped work.** [ORB-10268], [ORB-10269], [ORB-10271], and
  [ORB-10272] implemented the superseded model carefully. Most is deferred rather
  than deleted (§2.1), but "deferred" is a promise that costs maintenance and may
  never be collected — and [ORB-10272]'s substrate is deleted outright. That is the
  price of correcting the model at this point rather than later, and it is a real
  one.

## Task References

- [ORB-00424] — proposed the local/remote Orbit MCP unification (SSH-carried stdio,
  capability sets) that carries client→owner traffic. It is the whole cross-machine
  surface in v1; the hub→satellite half this design previously added is withdrawn.
- [ORB-10247] — implemented the versioned `HostIdentity` (§1): `schema_version` /
  `machine_id` / `host_id` / `mode`, `orbit init` ownership, legacy migration, and
  strict fail-closed loading (Phase 1 / Unit B1 under ORB-10246). This revision
  drops `mode` and adds `task_prefix` at `schema_version = 2`.
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
  ([Make orbit-registry the singular host/workspace registry domain crate](./4_decisions.md#make-orbit-registry-the-singular-hostworkspace-registry-domain-crate)).
- [ORB-10319] — widens and renames that extraction to the vertical `orbit-remote`
  feature: registry persistence, profile/cache/routine composition, MCP contract,
  broker, hub, link, and registration share one crate, while Store, MCP, Core,
  Tools, and Common remain neutral acyclic dependencies ([Consolidate remote host and MCP behavior in the vertical orbit-remote crate](./4_decisions.md#consolidate-remote-host-and-mcp-behavior-in-the-vertical-orbit-remote-crate)).
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
- [ORB-10725] — deleted [ORB-10272]'s allocation substrate and [ORB-10330]'s
  preallocated finalizers under [Workspace-scoped knowledge keys, no global knowledge IDs](./4_decisions.md#workspace-scoped-knowledge-keys-no-global-knowledge-ids): the hub sequence service, the
  reconciliation projection, the immutable ledger, the authority marker, and the
  `orbit/private/allocate-knowledge-id/v1` connector method are gone; Remote
  feature v2 is an empty slot and v3 drops its tables. Learning and ADR IDs are
  workspace-local, and the [ORB-10364] authoring gate is the single surface in
  front of a one-transaction owner-local write.
- [ORB-10730] — made the [Defer fleet registration and execution placement to v2](./4_decisions.md#defer-fleet-registration-and-execution-placement-to-v2) fleet boundary executable: v1 links only the
  local `host rename` command, serves workspace discovery from `workspaces.json`,
  ignores registry stamps and caches, and validates routine pins as own-host,
  belongs-elsewhere, or unresolvable from local owner names only.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
