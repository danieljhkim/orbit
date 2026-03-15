# Reduce Orbit-managed environment variable surface

## Goal
Stop Orbit from owning sensitive environment-variable concerns and tighten the execution-env contract so Orbit only manages the smallest safe set of variables.

## Scope
- audit every env var Orbit sets, passes through by default, or treats as required
- remove or redesign any handling that makes Orbit responsible for sensitive env vars
- clarify the contract in code, config defaults, and docs
- evaluate the later removal of HOME and PATH as an explicit hardening step

## Work items
1. Inventory the current env surfaces across runtime config, agent invocation, cli_command execution, tools, and docs.
2. Separate the model into three categories: Orbit-set vars, Orbit-pass-through defaults, and externally required provider/tool vars.
3. Implement the hardening change so sensitive env vars are not treated as Orbit-managed state.
4. Decide whether HOME and PATH can be removed safely now; if not, create a linked follow-up and document why.
5. Add tests that pin the resulting execution-env contract and update any affected docs/config assets.

## Done when
- Orbit no longer manages sensitive env vars
- the remaining env contract is explicit and tested
- HOME/PATH tightening is either safely implemented or captured as a concrete linked follow-up with rationale