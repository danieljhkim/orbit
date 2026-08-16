---
title: Orbit Core — Overview
owner: claude
last_updated: 2026-08-15
last_validated: 2026-08-15
status: Accepted
feature: orbit-core
doc_role: overview
type: design
summary: Crate boundary and public-surface contract of orbit-core after the ORB-10016 orbit-cmd extraction.
tags: [orbit-core, orbit-cmd, architecture]
paths: ["crates/orbit-core/**", "crates/orbit-cmd/**", "crates/orbit-registry/**"]
related_features: [orbit-core]
related_artifacts: [ORB-10016]
---

# Orbit Core — Overview

`orbit-core` is the runtime kernel of Orbit: it assembles config, stores,
policy, tools, and the event bus into the `OrbitRuntime` that consumers such
as `orbit-cli`, `orbit-web`, and `orbit-cmd` drive. This folder documents the
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
- **Registry-aware composition** — opening a selected registered checkout and
  composing routine discovery from local `host.toml` plus `workspaces.json`
  lives in `orbit-cmd::registry_runtime` and `orbit-cmd::registry_routines`,
  over persistence and validation owned by `orbit-registry`.
- **Consumer-justified re-export** — a root `pub use` kept only because a
  consumer crate (`orbit-cli`, `orbit-web`, `orbit-cmd`) genuinely
  imports it from the root.

## 3. At a Glance

| Concern | File | Task |
|---|---|---|
| Runtime bootstrap + roots | `crates/orbit-core/src/runtime/` | [ORB-10012] |
| Context assembly | `crates/orbit-core/src/context.rs` | — |
| Config layering | `crates/orbit-core/src/config/` | — |
| Runtime-integrated commands | `crates/orbit-core/src/command/` | [ORB-10016] |
| Registered runtime + routine composition | `crates/orbit-cmd/src/registry_runtime.rs`, `crates/orbit-cmd/src/registry_routines.rs` | — |
| Host identity + local workspace catalog | `crates/orbit-registry/src/` | — |
| Tool dispatch and command-audit boundary | `crates/orbit-core/src/command/tool/dispatch.rs` | — |
| Root re-export policy | `crates/orbit-core/src/lib.rs` | [ORB-10016] |
| Extracted command layer | `crates/orbit-cmd/src/` | [ORB-10016] |

## 4. Store Access Boundary

`OrbitStores` is Core's internal holder for typed `orbit-store` backends; it is
not an external SDK surface. Code inside Core delegates ordinary persistence
to the owning backend traits, while outer crates use explicit `OrbitRuntime`
methods and command APIs. Runtime methods may still coordinate several stores
or side effects when an operation is more than a single persistence call;
task mutation is one important example, not the only service wrapper.

## Task References

- [ORB-10012] — introduced the versioned workspace-layout migration pre-flight the runtime open runs.
- [ORB-10016] — extracted `orbit-cmd` from orbit-core and trimmed the root re-export surface.
- [ORB-10355] — removed the hand-maintained store delegation layer in favor of direct typed backend access.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
