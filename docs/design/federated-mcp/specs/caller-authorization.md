---
type: design
summary: "Spec: destination-side caller authorization — the callers file, requested-vs-granted authority, and key-bound caller identity"
last_validated: 2026-08-29
title: Spec — Destination-side caller authorization
owner: claude
status: Draft
feature: federated-mcp
tags: [federated-mcp, mcp, mcp-bridge, authorization, spec]
related_features: [federated-mcp, mcp-bridge, host-registry, mcp-session-context]
related_artifacts: [ORB-11044, ORB-11023, ORB-11017, ORB-11015, ORB-11013, ORB-11012, ORB-11010, ORB-11009, ORB-11008]
---

# Spec: Destination-side caller authorization

The authority an MCP session holds on a destination is **declared by that destination**, not requested by the caller. A remote-originated session's `--operator` argv becomes a *request*; the destination's machine-global callers file is the *ceiling*; the session's effective capabilities are the intersection. This contract is **proposed and not shipped**. It governs session authority (`agent` / `operator`) only, and does not touch capability class (`control_plane` / `execute`), which is already destination-derived and specified in [federated-workspace-mcp.md](./federated-workspace-mcp.md).

## Why This Exists

Two independent axes decide whether a federated call runs, and today only one of them is answered by the machine that executes the work.

| Axis | Question | Decided by, today |
|---|---|---|
| Capability class | Is this checkout a control plane, an execution binding, or neither? | The destination's own catalog role — `CapabilityClasses::for_checkout` in `crates/orbit-mcp/src/federated/capability.rs`, enforced before the tool body runs in `crates/orbit-cli/src/command/mcp/server.rs` |
| Session authority | May this caller perform governed operations at all? | The **caller's argv** — `--operator` on `orbit mcp serve`, resolved once at startup in `crates/orbit-mcp/src/remote/identity.rs` |

On an SSH destination the caller writes the remote argv. Any caller with shell access therefore writes `ssh <host> "orbit mcp serve --operator"` and stamps its own session `Operator`, which satisfies every entry in `GOVERNED_OPERATIONS` — `orbit.command.exec`, `orbit.workflow.ship`, `orbit.task.delete`, `orbit.workspace.claim.release`. The destination consults no caller identity when deciding this. `--remote-caller-machine-id` is an audit label the proxy forwards, explicitly "not an authenticated principal"; nothing reads it as an authorization input.

This is the unresolved transport-authentication question in [3_vision.md §1](../3_vision.md), narrowed to the part that can be answered without a fleet registry: not "who is this caller, globally" but "what has *this destination* agreed to serve *this caller*".

## Scope, and the boundary this does not claim

[`crates/orbit-common/src/governance/authorization.rs`](../../../../crates/orbit-common/src/governance/authorization.rs) states the governance kernel is an accident guard, not a security boundary, because every agent on a development box runs as one OS user and can bypass Orbit with `git` and `rm`. That reasoning is local. It does not carry across a machine boundary: a caller on another host does *not* already have shell on the destination except through SSH, and SSH authenticates.

This spec therefore has two tiers, and they must not be conflated:

- **Tier 1 — the callers file.** Moves the authorization *statement* to the destination. The caller identity it keys on is self-asserted, so Tier 1 alone remains an accident guard, in keeping with the kernel's doctrine. It is still strictly stronger than today, where the caller's request *is* the grant.
- **Tier 2 — key-bound caller identity.** Makes the identity authenticated by delegating to sshd. This is a real boundary for the remote case, and it introduces no Orbit-held credential: the key is the one SSH already checks.

Neither tier introduces a password, token, or keychain into Orbit. That prohibition stands.

## The callers file

Destination-side authorization is declared in the machine-global operator file `~/.orbit/mcp-callers.toml`. It is the mirror of `~/.orbit/mcp-destinations.toml`: destinations declare who this machine may call, callers declare who may call this machine and as what. Both files are machine-global; neither belongs in workspace `config.toml`, `workspaces.json`, or host-registry.

```toml
# Capabilities served to a caller that matches no row below.
# Permitted values: "agent" or "deny". Operator is never a default.
default = "agent"

[[callers]]
machine_id   = "hm_alpha"
label        = "daniels-mac-mini"
capabilities = ["agent", "operator"]

[[callers]]
machine_id   = "hm_beta"
capabilities = ["agent", "operator"]
workspaces   = ["ws_orbit", "ws_constellation"]
ssh_key_fingerprint = "SHA256:…"

[[callers]]
machine_id   = "hm_gamma"
capabilities = ["agent"]
```

Row keys:

| Key | Required | Meaning |
|---|---|---|
| `machine_id` | yes | The calling machine's stable `hm_…` identity. Matched against the caller identity resolved below. |
| `capabilities` | yes | The ceiling this destination will serve that caller. Values are `agent` and `operator` only. |
| `label` | no | Operator-facing display name. Never an identity input. |
| `workspaces` | no | Narrows the grant to these logical `ws_*` IDs. Omitted means every workspace on this destination. |
| `ssh_key_fingerprint` | no | Binds the row to an authenticated key. See Tier 2. |

File-level invariants:

1. A missing file is valid and means `default = "agent"` with no rows.
2. A duplicate `machine_id` makes the entire file invalid with `ambiguous_caller` at load, before any session is served — the same fail-closed shape `ambiguous_destination` has in the destinations file.
3. An unknown key, an unknown capability value, an empty `capabilities`, a `default` other than `agent` or `deny`, or a malformed `machine_id` invalidates the file at load. A malformed file is never served as if absent.
4. `runner` is not a grantable value. It is stamped in-process by a managed run and can never arrive over a transport.
5. Load happens once per server process, at startup, alongside identity resolution. A session's ceiling does not change under it mid-session.

## Remote origination is decided by the destination, not by argv

A session is **remote-originated** when the serving process observes `SSH_CONNECTION` in its own environment and standard input is not a terminal.

Both halves are load-bearing:

1. `SSH_CONNECTION` is set by sshd in the server process. A caller cannot forge it and, more importantly, cannot *omit* it. Keying on `--remote-caller-machine-id` instead would be bypassable by simply not passing the flag, which would present a remote session to the destination as a local one.
2. The non-terminal check separates the MCP transport from a person who SSH'd in and started a server by hand. The proxy argv is `ssh -T`, which never has a TTY; an interactive operator does. An operator whose interactive session is nonetheless piped is downgraded to the file's ceiling, which fails closed and is corrected by a row or by Tier 2.

A session that is not remote-originated is **local**, and its authority resolution is unchanged: the process owner is the caller, argv stays authoritative, and today's accident-guard model holds unmodified. No local workflow changes.

## Requested, granted, effective

For a remote-originated session:

```
requested = argv authority         (--operator ⇒ {agent, operator}; otherwise {agent})
granted   = matched row's capabilities, or the file default
effective = requested ∩ granted
```

Invariants:

1. **Intersection only.** The callers file can never raise a session above what its argv asked for. The change is downgrade-only in every direction, so it opens no new privilege path and cannot be used to escalate a session that did not ask.
2. **`--remote-caller-machine-id` selects a row; it never grants.** It stays an unauthenticated label. An absent, malformed, or unmatched label falls to `default` — it does not fall to the argv.
3. **`default = "deny"` yields the empty set**, and every governed operation therefore refuses, along with every ungoverned tool that requires `agent`. Ambiguity fails closed, matching `CallerCapabilities::resolve`.
4. **A `workspaces` narrowing is evaluated per call**, against the resolved workspace of that call, not at session establishment. A session may hold `operator` for one workspace and `agent` for another on the same destination.
5. **Resolution is session-only.** The MCP chokepoint's `CapabilityResolution::SessionOnly` is unchanged: `ORBIT_OPERATOR` in the destination's environment stays inert on the MCP surface, and must not become a way to re-raise a session the callers file capped.

## Tier 2: key-bound caller identity

Tier 1 leaves `machine_id` self-asserted. To make the caller identity authenticated, the destination pins it to the SSH key that authenticated, using a forced command in `authorized_keys`:

```
command="orbit mcp serve --accept-ssh --caller hm_alpha",no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding ssh-ed25519 AAAA… caller@daniels-mac-mini
```

Invariants:

1. Under a forced command the destination composes its own argv. The caller's command arrives only as `SSH_ORIGINAL_COMMAND` and **must be ignored entirely** — not parsed, not merged, not used to derive a requested authority. `requested` is then whatever the forced command names, which is the destination's own statement.
2. `--accept-ssh` marks the session remote-originated regardless of the environment checks above, and `--caller` supplies the identity. `--caller` is honored **only** in the presence of `--accept-ssh` under a forced command, and never as a caller-supplied flag on an ordinary `orbit mcp serve`.
3. When a row carries `ssh_key_fingerprint` and the destination can observe the authenticating key (`SSH_USER_AUTH`, or an `AuthorizedKeysCommand` that supplies it), a mismatch is a load-time-shaped refusal at session establishment, not a silent downgrade.
4. Tier 2 is opt-in. A destination running Tier 1 alone is a valid, documented configuration with a weaker guarantee, and the difference must be legible in the audit trail rather than assumed.

`orbit mcp callers authorize --machine-id <hm_…> --key <path>` emits the `authorized_keys` line. Orbit does not edit `authorized_keys` itself; the operator installs the line.

## Errors, audit, and diagnosis

1. A denial names the file, the resolved caller, and the requirement: `caller 'hm_alpha' is granted [agent] by ~/.orbit/mcp-callers.toml; 'orbit.command.exec' requires operator`. A refused caller must be able to act on the message without reading Orbit's source.
2. A new `CallerProvenance::RemoteGrant` distinguishes "the destination's callers file granted this" from a local `Session` stamp. Provenance is recorded, never discarded, so the trail can separate the two.
3. The audit envelope records the resolved caller identity, the granted set, and the effective set. Recording only the effective set would make a downgrade indistinguishable from a caller that never asked.
4. `orbit mcp callers list` prints the loaded rows and the default. `orbit mcp callers check <machine_id>` prints what a session from that caller would resolve to, without serving one.
5. Caller authorization is decided at **session establishment**, and the per-call `workspaces` narrowing at the existing governance chokepoint. Neither enters the routing precedence ladder in [federated-workspace-mcp.md](./federated-workspace-mcp.md), which classifies calls that have already been admitted.

## Surfaces this does not change

1. **`tools/list` does no capability filtering.** Placement is not permission; that invariant is unchanged, and a capped session still sees the full advertised surface and is refused at the chokepoint.
2. **The mux's client side.** `--mode federated` requests `agent` and should continue to. The mux is a caller here, and its request is capped by each destination independently.
3. **`orbit mcp listen`.** A TCP socket authenticates nobody, so its hardcoded `Agent` authority stays correct and must not be made configurable by this file.
4. **Capability class.** `control_plane` / `execute` and `capability_refused` are untouched. The two axes compose: a call must clear both.
5. **v1 `--mode remote` byte-transparency.** The proxy is unchanged. The destination-side resolution happens in the server the proxy reaches, not in the proxy.

## Migration

A missing callers file must not silently preserve today's behavior, because today's behavior is the defect.

1. First release: a missing file means remote-originated sessions resolve to `agent` only, and the server emits one warning at startup naming the file to create. This is a downgrade that **will** cut existing operator-over-SSH flows on the first upgrade, and that is the intended direction.
2. `orbit mcp callers init` seeds a file from the `machine_id`s already present in the local registry and destinations file, granting `agent` and leaving `operator` for the operator to add deliberately. Adding `operator` must remain an explicit edit; the seeder must never write it.
3. `orbit doctor` reports a destination that serves SSH sessions with no callers file, and a row granting `operator` with no `ssh_key_fingerprint` when Tier 2 is available.
4. There is no compatibility window in which the caller's argv is honored on a remote-originated session. A phased default would leave the escalation path open for the length of the phase while implying it was closed.

## Agent Signature

Drafted by claude, 2026-08-29, from a read of `crates/orbit-mcp/src/remote/identity.rs`, `crates/orbit-mcp/src/federated/`, `crates/orbit-cli/src/command/mcp/`, and `crates/orbit-common/src/governance/authorization.rs`. Proposed, unimplemented; the implementing task records its decision entry in [4_decisions.md](../4_decisions.md) alongside the code.
