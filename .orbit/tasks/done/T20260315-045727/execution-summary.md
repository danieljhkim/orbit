Consolidated the default identity pipeline from six overlapping personas down to four canonical defaults and made the runtime understand the new architect/reviewer roles end to end.

Summary of changes:
- added `architect` and `reviewer` to `IdentityRole`, including parse/display support and CLI help text
- replaced the seeded default identity set with `linus`, `lamport`, `prii`, and `steve` in both bundled assets and tracked `.orbit/identities` copies
- updated `prii` from maintainer/leader framing to reviewer framing, while keeping `steve` as `ceo`
- repointed bundled and tracked `resolve-backlogged-task` activity specs from `kent` to `linus`
- refreshed CLI tests so init now expects four default identities and identity commands explicitly cover the new roles

Strategic decisions:
- added real runtime role support for `architect` and `reviewer` instead of trying to shoehorn those personas into old role names | Rationale: the replacement identity set should load honestly and be filterable through existing CLI/runtime surfaces | Trade-offs: slightly larger role enum surface, but much less semantic drift in the identity files
- kept historical task bundles and old task metadata untouched even though they still mention retired identities | Rationale: those records are audit history, not live defaults | Trade-offs: repo history still contains old names, but runtime/default sources are now consistent

Assumptions made:
- `linus` is the correct replacement execution identity for `resolve-backlogged-task` | Impact if incorrect: the activity would still resolve, but its persona choice would need product-level reconsideration

Design weaknesses / risks:
- tracked `.orbit/activities/active` files are not simple mirrors of bundled activity assets; some still diverge structurally beyond the `identity_id` field | Severity: Medium | Mitigation: follow-up issue `T20260315-050412` was created to resolve the source-of-truth drift

Deviations from original plan:
- did not change `steve.yaml` beyond keeping the file in the new four-identity set | Justification: the existing CEO persona already matched the intended outcome

Technical debt introduced:
- None beyond the pre-existing bundled-vs-active activity drift captured in `T20260315-050412`

Recommended follow-ups:
- resolve `T20260315-050412` so built-in activity assets and tracked active copies stop drifting semantically

Validation:
- `cargo test -p orbit-cli --test identity_commands`
- `cargo test -p orbit-cli --test init_commands`
- `cargo test -p orbit-core -- --nocapture`
- `cargo build --workspace`
- `cargo run -q -p orbit-cli -- identity list`
- `cargo run -q -p orbit-cli -- identity show linus --json`
- `cargo run -q -p orbit-cli -- identity show lamport --json`
- `cargo run -q -p orbit-cli -- identity show prii --json`
- `cargo run -q -p orbit-cli -- identity show steve --json`
- `rg -n "grace|john|kent|rob" orbit-core/assets/identities orbit-core/assets/activities .orbit/identities .orbit/activities/active`