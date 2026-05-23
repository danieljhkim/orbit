## Context

Orbit-docs frontmatter needs a way to cross-link from a doc to any other allocation-bearing artifact: a task (`ORB-NNNNN`), a learning (`L-NNNN`), a friction (`F<YYYY>-<MM>-<NNN>`), or an ADR (`ADR-NNNN`). The candidate shapes were (a) an array of `{type, id}` objects, (b) a single ambiguous `references` field, or (c) ID-prefix dispatch over a flat string array.

## Decision

`related_artifacts` is a flat string array. The parser dispatches on the ID prefix to type the reference: `ORB-` to task, `L<digits>-<digits>` to learning, `F<digits>-<digits>-<digits>` to friction, `ADR-` to ADR. Unknown prefixes are a hard parse error (not silently kept as opaque strings).

## Consequences

- Frontmatter stays human-writable: `related_artifacts: [ORB-00163, ADR-0168]` is shorter and more skimmable than `[{type: task, id: ORB-00163}, {type: adr, id: ADR-0168}]`.
- The set of dispatchable prefixes is closed at parser-extension time, not at frontmatter-author time. Adding a new artifact kind (e.g. `M` for memory) requires editing the parser and adding a test, not negotiating with every doc author's frontmatter.
- Strict-unknown-prefix matters: silent acceptance of `XYZ-1` would let typos rot in the corpus undetected (`OBR-00163` instead of `ORB-00163`) and become broken cross-refs only at injection time. Hard erroring on parse forces the typo to surface at `orbit docs migrate`/`list`/`show` time, when there's a human reviewing.
- Cost: the prefix grammar is now load-bearing across orbit. The day Orbit changes task IDs from `ORB-NNNNN` to a different shape (say a UUID or a longer numeric range), the parser changes too — and so does any frontmatter already on disk. This is the same coupling cost the rest of orbit's ID conventions already pay; this ADR makes it explicit for orbit-docs's slice.