---
title: Orbit MCP — Vision
owner: codex
last_updated: 2026-08-15
last_validated: 2026-08-15
status: Draft
feature: mcp-bridge
doc_role: vision
type: design
summary: Evidence-gated extensions beyond the deliberately small direct-SSH MCP v1.
tags: [mcp, ssh, authorization, audit]
paths: ["crates/orbit-mcp/**", "crates/orbit-core/**", "crates/orbit-registry/**"]
related_features: [host-registry, mcp-session-context]
---

# Orbit MCP — Vision

V1 is a skeleton: local stdio, direct SSH stdio, server-side resolution, one Core
dispatch boundary, and audit context. Future work should preserve that shape and
add complexity only for a demonstrated requirement.

## 1. Authorization in Core

The next security layer belongs in Core, not in the local proxy, client UI, or
tool advertisement path. A useful authorization design must first answer:

- What authenticated principal does the accepting server receive?
- How is that principal bound to a machine or operator?
- Which rules apply globally and which apply per workspace or operation?
- How are denials audited without creating a second dispatch path?

The v1 `caller_machine_id` and `caller_ip` fields are insufficient for enforcement.
The label is supplied by the caller and an IP address is neither stable nor a
machine credential. Possible inputs include SSH certificate principals, forced-
command metadata, or another server-verified identity, but no mechanism should be
chosen before the required trust model is concrete.

## 2. Better provenance

Audit records may eventually need the authenticated SSH principal, key fingerprint,
connection identifier, or proxy version. Each added field should have a clear
source of trust and retention purpose. Audit enrichment must not silently turn an
observational field into an authorization key.

## 3. Contract skew

Direct SSH can connect different Orbit versions. Standard MCP discovery already
lets the accepting server advertise its actual surface. Add explicit compatibility
negotiation only if mixed-version failures cannot be explained cleanly through
normal discovery and errors.

## 4. Other transports

SSH is appropriate while every supported remote has a shell and the same operator
controls both ends. A network MCP service would require Orbit-owned authentication,
session lifecycle, exposure hardening, and operational support. Do not add one
merely to avoid launching SSH.

## 5. Multi-host routing

The v1 client chooses one destination and that server answers only from its own
state. Automatic owner discovery, relays, replication, or fleet placement should
be considered separate products, not incremental proxy features. Each needs a
measured use case strong enough to justify new authority and failure modes.

## 6. Evaluation gates

Any extension should preserve these properties unless the change explicitly
replaces them:

1. one canonical tool schema;
2. one authoritative accepting server;
3. one Core dispatch and audit boundary per tools/call, including unknown names;
4. no policy in byte-forwarding transport;
5. no client-side check required for correctness; and
6. a focused end-to-end test for local and remote context propagation.

Current behavior is described in [`2_design.md`](./2_design.md). The machine-
readable contract is [`references/conformance-v1.yaml`](./references/conformance-v1.yaml).
