## Context
The current runtime persists a chronicle plus runner state. That works, but it does not cleanly separate "what later prompts should load" from "what operators should inspect after the fact."

## Decision
The intended Groundhog direction is two persisted views: prompt-facing success memory and audit-only run history.

## Consequences
- Prompt loading rules become simpler and harder to accidentally violate.
- Audit surfaces can grow richer without bloating prompt state.
- Cost: migrating from today's `Chronicle` plus state artifact will require a persistence rewrite and compatibility plan.

---

## Task References

- **[T20260420-0509]** — Add Groundhog chronicle serializer and shared Groundhog data types.
- **[T20260420-0509-2]** — Add structured task plan parsing with typed checkpoints and success criteria.
- **[T20260420-0509-3]** — Add Groundhog builtin verb tools.
- **[T20260420-0509-4]** — Add Groundhog workspace snapshots and scratch-branch rewind mechanics.
- **[T20260420-0510]** — Add the shared runtime checkpoint verifier.
- **[T20260420-0510-2]** — Add the Groundhog v1 activity runner.
- **[T20260426-0603]** — Remove the public Groundhog checkpoint deviation verb from the tool surface.
- **[T20260430-21]** — Shorten Groundhog design docs and add missing ADR task citations.
- **[T20260509-19]** — Split the Groundhog activity runner into focused engine submodules.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
