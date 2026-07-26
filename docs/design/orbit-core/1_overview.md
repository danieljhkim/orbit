---
title: Orbit Core — Overview
owner: claude
last_updated: 2026-07-25
status: Accepted
feature: orbit-core
doc_role: overview
type: design
summary: Crate boundary and public-surface contract of orbit-core after the ORB-10016 orbit-cmd extraction.
tags: [orbit-core, orbit-cmd, architecture]
paths: ["crates/orbit-core/**", "crates/orbit-cmd/**"]
related_features: [orbit-core]
related_artifacts: [ORB-10016, ADR-0203]
---

# Orbit Core — Overview

`orbit-core` is the runtime kernel of Orbit: it assembles config, stores,
policy, tools, and the event bus into the `OrbitRuntime` that every consumer
(CLI, dashboard, extracted command layer) drives. This folder documents the
crate's boundary — what belongs inside the kernel, what was extracted to
`orbit-cmd` in [ORB-10016], and the contract governing its root re-exports.

## 1. Motivation

orbit-core had grown into a god-crate SDK (~46k LoC, 178 files): runtime
bootstrap, config layering, an 18-submodule `command/` tree, the agent tool
hosts, and ~23 root `pub use` groups re-exporting on the order of 120 items.
Consumers could not tell which surface was load-bearing, and every command
change recompiled the whole kernel. [ORB-10016] split the CLI-facing command
layer into `orbit-cmd` and trimmed the root surface to what consumers
demonstrably import.

## 2. Core Concepts

- **Runtime kernel** — `OrbitRuntime` construction: root resolution, config
  layering, store/policy/tool wiring, default asset seeding, event bus.
- **Runtime-integrated command** — a command module the kernel itself invokes
  (from the `orbit.*` tool hosts, the engine hosts, or bootstrap seeding).
  These stay in `orbit-core::command`.
- **CLI-facing command** — a command module that is a pure consumer of the
  runtime's public API. These live in `orbit-cmd` and attach their runtime
  methods via `*Commands` extension traits.
- **Consumer-justified re-export** — a root `pub use` kept only because a
  consumer crate (`orbit-cli`, `orbit-dashboard`, `orbit-cmd`) genuinely
  imports it from the root.

## 3. At a Glance

| Concern | File | Task |
|---|---|---|
| Runtime bootstrap + roots | `crates/orbit-core/src/runtime/` | [ORB-10012] |
| Context assembly | `crates/orbit-core/src/context.rs` | — |
| Config layering | `crates/orbit-core/src/config/` | — |
| Runtime-integrated commands | `crates/orbit-core/src/command/` | [ORB-10016] |
| Root re-export policy | `crates/orbit-core/src/lib.rs` | [ORB-10016] |
| Extracted command layer | `crates/orbit-cmd/src/` | [ORB-10016] |
| Crate-boundary ADR log | [4_decisions.md](./4_decisions.md) | [ORB-10016] |

## 4. Store Access Boundary

[ORB-10355] removed orbit-core's hand-written per-domain store forwarding
layer. `OrbitStores` now exposes each typed `orbit-store` backend directly, so
adding a method to a backend trait makes it available to core callers without
adding a matching forwarding method. Callers use the backend's canonical
method names, which also keeps the persistence contract searchable from the
call site.

Direct exposure was chosen over macro-generated forwarders because a macro
would still maintain a second method inventory in orbit-core and could drift
from the owning trait. The only retained service is task-record mutation:
creating, updating, and deleting a task coordinates multiple task backends
with semantic-index side effects, so those operations are orchestration rather
than pure delegation. Read-only task access goes directly to the relevant
typed backend like every other domain.

## Task References

- [ORB-10012] — introduced the versioned workspace-layout migration pre-flight the runtime open runs.
- [ORB-10016] — extracted `orbit-cmd` from orbit-core and trimmed the root re-export surface.
- [ORB-10355] — removed the hand-maintained store delegation layer in favor of direct typed backend access.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
