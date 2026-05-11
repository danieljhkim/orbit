## Context
A separate "deny pass" before profile evaluation is the obvious shape, but it makes precedence ambiguous when a profile rule and a deny rule both match. Multiple Orbit features (workspace overrides, profile narrowing, denyModify-also-implies-denyRead-for-modify validation) need a single evaluation order.

## Decision
`effective_profile` appends every entry of `denyRead` to the profile's `read` list as `!<rule>` and every entry of `denyModify` to the profile's `modify` list as `!<rule>`. `check_path` walks the resolved list in order and the **last match wins**. There is no separate deny pass.

## Consequences
- Profile rules and deny rules are evaluated in one deterministic pass; appended denies win over earlier positive matches.
- Cost: a profile author cannot re-allow a globally denied path by ordering, which is the intended safety property but surprises authors who expect a simple allowlist with overrides.
