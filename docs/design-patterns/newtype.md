---
type: pattern
summary: "Newtype Wrapper"
---
# Newtype Wrapper

Wrap a primitive (typically `String` or a numeric type) in a one-field struct with a private inner value and a *fallible* constructor that validates. Downstream code accepts the newtype and trusts the value is well-formed — invariants are enforced once, at the boundary.

```rust
pub struct Wrapper(Inner);

impl Wrapper {
    pub fn new(value: Inner) -> Result<Self, Error> {
        validate(&value)?;
        Ok(Self(value))
    }
    pub fn as_inner(&self) -> &Inner { &self.0 }
}
```

Sometimes phrased as "parse, don't validate": validate once at construction so no downstream caller has to.

## When to reach for it

- **A primitive carries protocol meaning.** Git ref names, content-addressed hashes, semantic-version strings, IDs — each has a well-formedness contract that callers shouldn't be re-checking.
- **The same value is passed through many layers.** Once a function receives `&str` it has no idea whether the caller validated. A typed wrapper carries the proof.
- **You're tempted to write `is_valid_X(&str) -> bool`.** That's the smell: validate-then-use leaves room for time-of-check-time-of-use bugs and forces every callsite to remember the check.

## When NOT to

- **The value never leaves one function.** Local strings don't need typing; validate inline.
- **The "primitive" is already typed.** `PathBuf`, `Uuid`, `chrono::DateTime` already wear their domain.
- **The primitive really is free-form.** A user-facing note, comment, or description has no contract to enforce.

## Reference: `RefName`

A git-ref-name wrapper around `String`. From `crates/orbit-knowledge/src/graph/object_store.rs:27`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RefName(String);

impl RefName {
    pub fn new(value: impl Into<String>) -> Result<Self, KnowledgeError> {
        let value = value.into();
        validate_ref_name(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str { &self.0 }
}

impl fmt::Display for RefName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
}

fn validate_ref_name(value: &str) -> Result<(), KnowledgeError> {
    if value.trim().is_empty() { /* ... */ }
    if value.starts_with('-') { /* ... */ }
    if value.starts_with('/') || value.ends_with('/') || value.contains("//") { /* ... */ }
    // ...remaining git ref-name grammar checks...
    Ok(())
}
```

Downstream code accepts `&RefName` or `RefName` and never re-checks.

Patterns to copy:

- **Tuple struct with a non-`pub` inner field.** `pub struct RefName(String)` lets you derive `Debug`/`Clone`/`Hash`/`PartialEq`/`Eq` for free while keeping outside callers from constructing `RefName("--bad".into())` directly.
- **`new(impl Into<String>) -> Result<Self, _>` as the only constructor.** Accepts `&str`, `String`, anything string-like; returns the typed error if invalid. Don't add a `From<&str>` impl (it would have to panic) or `pub const` constructors.
- **`as_str(&self) -> &str` for read-only access.** Callers needing the raw form (display, serialization, passing to APIs that take `&str`) get it cheaply, without enabling mutation.
- **Validation in a private free function next to the type.** Not a `validate()` method on the wrapper — the wrapper *carries* the proof of validation; it doesn't re-offer it.

Use this shape whenever a primitive has a protocol contract worth enforcing once at the boundary.

---

**Related: parsed sum-type form.** `Selector` (`crates/orbit-common/src/utility/selector.rs:16`) applies the same principle to a structured input string: `FromStr` parses `dir:`/`file:`/`symbol:` prefixes into an enum with a typed `SelectorParseError`. The enum variants have `pub` fields (a deliberate ergonomics trade for pattern matching at use sites), so the invariant is enforced by convention — every callsite uses `parse()` — rather than by visibility. Reach for this when the input has multiple legitimate shapes and pattern-matching the parsed result outweighs airtight construction.
