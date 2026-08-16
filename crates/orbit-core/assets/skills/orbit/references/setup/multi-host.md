# Running Orbit on more than one machine

Two machines working the same repository is the normal case: a laptop and a
build box, a workstation and a server. Orbit supports it, but three things
collide by default and have to be arranged deliberately.

## The mental model

Orbit does **not** synchronize state between machines. It synchronizes
*definitions*, through git, because they are files in the repository:

| Synced through git | Host-local, never synced |
|---|---|
| `.orbit/config.toml` | Host identity (`~/.orbit/host.toml`) |
| `.orbit/routines/*.yaml` | Routine last-fire times and pauses |
| `.orbit/auto_tasks/*.yaml` | Run history and job-run bundles |
| `.orbit/resources/` | Task locks and reservations |
| Task bundles, once committed | The audit store, logs, and search indexes |

Two machines run the same routine definitions against completely independent
scheduler state. Nothing coordinates them. That is the source of every surprise
below.

## 1. Task-ID collisions

Every machine allocates task IDs from its own counter. Two machines with the
same prefix will allocate the same ID for different work, and there is no
reconciliation. Two ways to keep them apart — pick one, per repository:

**Distinct prefixes (recommended).** The task prefix is chosen at `orbit init`,
is 2–5 uppercase ASCII letters, and is **immutable** — there is no rename. Give
each machine its own, so an ID says where it came from and collision is
structurally impossible.

```bash
orbit init --non-interactive --host-name <name> --task-prefix <PREFIX>
```

Get this right on the first init of each machine. Reserved namespaces are
refused, and the value cannot be changed afterward.

**Disjoint numeric ranges.** When machines must share a prefix, hand each a
different floor:

```bash
orbit workspace init --task-id-start 10000     # this machine allocates from 10000 up
```

Also settable later as the `tasks.id_start` config key. The counter only moves
forward; a value below the current position is refused. This is more fragile
than distinct prefixes — a range large enough to never be exhausted is a guess —
so prefer prefixes and keep this for machines that were initialized before the
question came up.

## 2. Routine collisions

Routines have no "any host" mode. Every definition pins explicit hosts:

```yaml
hosts: [<host-id>]
```

Because definitions sync but state doesn't, an unpinned-to-you routine is simply
inert on your machine — which is the desired behavior, and why pinning is
mandatory rather than optional. Add a second host ID to the list only when you
genuinely want both machines running that pipeline, and then think about whether
`overlap: forbid` is enough (it is per-host, so it is not).

Routine *names* must be unique across every routine source on a host, which is
why seeded routines carry a workspace-derived suffix.

Host names come from `orbit init`, and can be changed later:

```bash
orbit host rename <current-name> <new-name>
```

This updates `host.toml` and the local workspace owner records. It does **not**
rewrite `hosts:` pins in versioned routine definitions — update those in the
same commit, or the routine goes quietly inert.

## 3. Dispatch collisions

Two machines shipping the same backlog will both pick up the same tasks. Task
locks and reservations are host-local, so they do not arbitrate across machines.

Use a workspace claim: one operator holds it, the other presents its token.

```bash
orbit run ship --claim-token <token>
ORBIT_WORKSPACE_CLAIM_TOKEN=<token> orbit run auto --for 1h
```

The simpler arrangement, where it fits the work: give each machine a different
*role* rather than a shared backlog. One ships, the other runs read-only
routines like task-pilot and triage. → [orchestration.md](../orchestration.md)

## Verifying a second machine

After initializing machine two:

```bash
orbit workspace show          # is the checkout registered here?
orbit routine list            # which routines are pinned to this host, and enabled?
orbit sweep --dry-run         # what would fire here
orbit doctor
```

`orbit routine list` is the important one. A routine that is enabled in the
versioned definition but not pinned to this host shows as such, which answers
"why didn't it fire here" before you go looking in logs.

## Reaching one machine from another

Nothing above moves state between machines. To *look at* or *drive* another
machine's Orbit — its dashboard or its tool surface — see
[remote-access.md](remote-access.md). Remote access is live access to one
machine's state, not replication.
