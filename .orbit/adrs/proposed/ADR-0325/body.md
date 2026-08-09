## Context

The Linux sandbox executor launches agent processes under a user-namespace sandbox. On hosts where unprivileged user namespaces are restricted, the sandbox helper cannot establish its uid map and every dispatch fails at spawn with a permission error that reads, to anyone above the spawn layer, like an executor defect.

A per-dispatch preflight now exists — it actually executes the sandbox helper rather than merely checking that the binary is installed — and dispatch fails closed with a remediation hint when the probe fails. Executors may declare an opt-in that permits running without the sandbox, and that opt-in defaults off.

What is still unrecorded is the stance behind all of this. Two questions have no written answer:

1. When the host cannot provide the sandbox, is the correct outcome to refuse, or to degrade?
2. Is making the host capable an operator responsibility, or is it Orbit's job to work anywhere?

Without an answer, each incident is re-litigated from scratch. The most recent occurrence appears to have been cleared out of band, with no record of what was changed or by whom — which is the concrete cost of having no stated position. A related gap follows from the same silence: the opt-in exists as a declarable field with no documented operator procedure telling anyone when it is legitimate to set it, so its only realistic uses are panic and cargo-culting.

The alternatives considered were: making the sandbox best-effort with automatic fallback to an unsandboxed run; attempting to detect and work around restricted namespaces inside the sandbox helper by emitting a different argument shape; and treating sandbox availability as a host precondition.

## Decision

Treat sandbox availability as a host precondition. Orbit verifies it and refuses; it does not silently degrade.

Concretely: the preflight stays fail-closed and stays per-dispatch. When it fails, the failure is permanent and names the host condition rather than the symptom. Running without the sandbox remains possible only through the explicit per-executor opt-in, which stays defaulted off and is documented as an operator decision with a stated risk, not a troubleshooting step. Orbit does not attempt to detect restricted namespace configurations and emit an alternative argument shape to work around them.

Making a host capable of running the sandbox is an operator responsibility, and the supported remedies belong in the runbook rather than in code.

## Consequences

- A host that cannot sandbox fails loudly and early, at a layer that knows why, instead of producing spawn errors that read as executor or agent defects.
- The opt-in acquires a meaning: it marks a deliberate, recorded acceptance of running unsandboxed on a specific executor, rather than an undocumented escape hatch.
- Out-of-band host changes stop being invisible, because the runbook names what a correct host looks like and the preflight asserts it.
- Cost: an operator whose host cannot be reconfigured — a managed or hardened environment where unprivileged user namespaces are unavailable and cannot be enabled — has no supported path except opting individual executors out of sandboxing entirely. This decision deliberately refuses the middle ground of automatic degradation, which means that operator carries a coarser, more explicit risk than a fallback would have given them.
- Cost: the per-dispatch probe executes the sandbox helper on every dispatch. That is a real cost paid on every run to catch a condition that changes rarely, accepted because a stale cached answer is worse than the probe.