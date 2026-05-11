## Context
Activities can omit `fsProfile:`. A naive design would either reject the activity at load or run it without policy enforcement. Both are wrong: rejection breaks the common case, and unguarded execution means audit blindness.

## Decision
When an activity omits `fsProfile:`, the v2 host substitutes the constant `UNRESTRICTED_FS_PROFILE` ("unrestricted") at `tool_context_for_activity`. If the policy does not define a profile of that name, the resolver synthesizes `read: ["./**"]` and `modify: ["./**"]`. Global `denyRead` / `denyModify` rules still apply because they are injected after profile resolution.

## Consequences
- "Unrestricted" remains auditable and narrowed by global denies, while policy authors can shadow it with a real profile.
- Cost: the word "unrestricted" carries different meaning depending on whether the policy defines a profile of that name, which is a learnable but real source of confusion.
