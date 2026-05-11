## Context
Leaf node IDs are derived from `symbol:{path}#{qualified_name}:{kind}`. Python methods in different classes, Rust inherent/trait impls for the same type, Java overloads, and TypeScript overloads can naturally produce duplicate `(qualified_name, kind)` pairs within one file, causing SQLite fast paths keyed by `node.id` to drop leaves that the JSON fallback preserves.

## Decision
Require every extractor to emit per-file-unique leaf IDs using the scheme specified in [specs/leaf-id-uniqueness.md](./specs/leaf-id-uniqueness.md): first add language-natural qualifiers, then run a universal deterministic occurrence-suffix finalizer that patches parent/child qualified-name references. The SQLite index schema is bumped when this lands so old sidecars fall back instead of mixing old duplicate-prone IDs with new reads.

## Consequences
- SQL and JSON fallback paths can compare leaves as multisets without accepting duplicate-ID carve-outs.
- Python, Rust, Java, and TypeScript selectors for affected methods, impls, and overloads change after rebuild; callers must treat the qualified portion of symbol selectors as opaque.
- Parent-child wiring remains stable only if the finalizer patches relation strings after suffixing, so extractor tests must cover that failure mode.
- Cost: selector strings for affected leaves are a breaking change, and occurrence suffixes are less readable than pure language syntax when two declarations remain indistinguishable after natural qualification.

---
