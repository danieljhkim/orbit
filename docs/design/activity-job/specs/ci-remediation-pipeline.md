---
type: design
summary: "Spec: host-owned CI discovery and candidate verification around the implementing agent"
tags: ["activity-job"]
last_validated: 2026-08-30
---

# Spec: CI Remediation Pipeline

`task_ci_remediation_pipeline` guarantees that a CI-failure repair is
**discovered from real GitHub state before an agent is launched** and
**verified green on the exact published commit before the task is promoted**.
Neither guarantee is asked of the implementing agent, because neither is
possible where it runs.

## Why This Exists

The `ci-failure-remediation` auto-task asked its agent for two things its
execution lane structurally cannot deliver.

1. **Discovery needs credentials the sandbox denies.** The `github.*` builtins
   run `gh` as a child of whichever process executes the tool. A lane that
   denies reads of the GitHub CLI's credential directory and forwards no token
   cannot authenticate, so discovery can only report
   `capability_unavailable` — an honest failure, but not a working lane.
2. **Post-publication verification happens before publication.**
   `agent_implement` is the implement step of the delivery pipeline, which runs
   *before* commit → prepare → sync → push → PR open. The candidate commit does
   not exist while the agent is running. A requirement for "a green rerun on the
   candidate commit" can only be hung on or fabricated.

Both belong on the host. Engine-private shipment automation already runs `gh`
unsandboxed on a boundary that is never advertised to agents and does not pass
through tool authorization or activity allowlists
(`orbit-engine/src/executor/automation/vcs/operations.rs`); CI discovery and
candidate verification sit on that same boundary
(`orbit-engine/src/executor/automation/ci/`).

## Stages

| Stage | Kind | Guarantee |
| --- | --- | --- |
| `collect_ci_evidence` | deterministic, host | Preflight plus every CI query runs on the host. Emits one bounded, redacted snapshot. |
| `classify_ci_evidence` | deterministic, host | Triages the snapshot into exactly one of three outcomes. |
| `agent_implement` | agent loop | **Unchanged.** Receives `ci_evidence` through the job input. Ordinary tool baseline; no `github.*`, no credential, no sandbox carve-out. |
| `git_commit` … `pr_open` | deterministic | The shipped delivery chain, reused as-is. |
| `verify_candidate_ci` | deterministic, host | Bounded wait for workflows on the exact candidate SHA. Promotion is unreachable unless green. |
| `pr_promote` | deterministic | The shipped promotion, reused as-is. |

## The Evidence Snapshot

`collect_ci_evidence` derives heads rather than assuming branch names: the
integration head is the run's own base branch, the release head is whatever
GitHub reports as the repository default (`gh repo view`), and pull-request
heads come from `gh pr list`. Each is resolved to a current SHA with
`git ls-remote`; when the two branches coincide the ref is scanned once.

Three commits that are routinely conflated stay in separate fields on every
reported run:

- `event_reported_head_sha` — what the workflow event carried. Metadata.
- `current_ref_head_sha` — what the ref points at now.
- `actual_checkout_shas` — what the runner actually checked out, parsed from the
  runner's own log. Evidence.

A failure at a SHA the ref has moved past is `advanced_head`; a failure with a
later successful run of the same workflow at the same SHA is
`superseded_by_success`, citing the superseding run. Age alone is never
staleness evidence. Runs without a verdict at a current head are `in_flight`,
not failures.

**Truncation is reported, never silent.** `truncation` carries every bound the
collection hit — refs scanned, runs listed, failures discovered versus
investigated, log byte cap, checkout-log reads spent — plus prose notes for any
head that could not be resolved or any pull-request page that hit the cap. The
agent cannot issue a follow-up query, so it must never have to guess whether
"no more failures" meant "none" or "we stopped looking".

Everything crossing into the sandbox is bounded and redacted through
`orbit_tools::github_cli`. No token, host configuration, arbitrary command, or
caller-selected environment crosses with it.

## The Three Endings

These must never collapse into each other or into a clean pass.

- **`capability_unavailable`** — no GitHub client, or no usable credentials.
  Triage moves the bundle to `blocked` carrying the exact preflight detail, then
  fails the step, which ends the run before an agent is dispatched. No agent run
  is spent. A failed run is the honest report: the host was asked whether CI is
  red and could not find out.
- **`no_current_failure`** — the queries ran and nothing current is failing.
  Triage persists the evidenced account as the task's durable
  `execution_summary`, the agent is skipped, and `git_commit` finds no diff on a
  task carrying `no-diff-expected`. That is the shipped no-diff promotion route,
  entered with no agent run and no change to any delivery stage.
- **`current_failures`** — implement, publish, verify, promote.

## Candidate Verification

`verify_candidate_ci` filters workflow runs by `reported_head_sha` against the
commit `git_push` actually published (`push.output.local_sha`), not the branch
it landed on — a branch can move again before CI reports. Every run on that SHA
is affected, which is how informational checks are covered alongside required
ones.

Verdicts, all distinguishable:

| Verdict | Meaning |
| --- | --- |
| `green` | Every affected workflow completed `success` / `skipped` / `neutral`. |
| `red` | At least one `failure` / `timed_out` / `action_required` / `startup_failure`. |
| `cancelled` | No red, but a run was cancelled. Not a pass, and not a failing test. |
| `queued`, `in_progress`, `missing` | Unsettled, reported when no wait budget was offered. |
| `wait_timeout` | The budget ran out while unsettled. `pending_state` names which. **Not a CI failure.** |

Only `green` returns. Every other verdict records the whole structured result as
a durable task comment, moves the bundle to `blocked` with a bounded headline,
and fails the step — so promotion is unreachable rather than merely skipped.

Feeding a red candidate's new failure logs back into a bounded repair iteration
is out of scope; `failure_evidence` is shaped to carry what that iteration would
need.

## Routing

`task_gate_pipeline` dispatches `task_{{ input.mode }}_pipeline`, but `mode` is
a run-level input. `list_backlog_tasks` now emits `dispatch_bundles`, pairing
each bundle with the mode it must ship through, derived at the one place that
holds the task record: a `ci-failure-remediation`-tagged task under `pr` mode
becomes `ci_remediation`. The override applies only on top of `pr`, because this
pipeline publishes and then verifies a candidate and has nothing to refine when
the caller asked for local-only delivery.

`validate_bundles` fails closed on any routing that could mis-ship work: one
entry per bundle covering the same task ids, and a bundle routed off the default
mode must contain exactly one task. Ordinary `pr` and `local` dispatch is
unchanged, including when no `dispatch_bundles` is supplied at all.

## Structural Constraint: `when:` Renders Before It Evaluates

A step condition is rendered through the template engine and *then* evaluated,
and a skipped step records no output. Any step whose output a later `when:`
reads must therefore run unconditionally. That is why this pipeline ends the run
at `triage` on `capability_unavailable` instead of skipping its way down, and
why `commit` is unconditional: `triage` and `commit` are the only two step
outputs any condition reads.

## Not In Scope

- The shipped `ci-failure-remediation` auto-task is unchanged: its definition,
  its five `github.*` `required_tools`, and its `enabled: false` state all
  remain. It is retired only once this pipeline is proven a replacement.
- No host broker, `host_brokered` tool classification, or loopback IPC. It does
  not fix the pre-publication problem at all, and it would add a privileged
  transport and a second tool-authorization enforcement point to serve five
  read-only calls the host can already make directly.
- No GitHub credential is granted to, or forwarded into, the agent sandbox.
- The child-side reconstruction of the tool allowlist from
  `ORBIT_ACTIVITY_TOOLS` is a separate weakness and is untouched.
