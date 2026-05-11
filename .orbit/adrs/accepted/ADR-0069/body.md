## Context
Tree-sitter extractors covered five source-code languages; every other file landed in the graph as a leafless `FileNode`. Design docs under `docs/design/` and scoreboard/config files under `.orbit/` were load-bearing context but invisible to graph queries at sub-file granularity. The `LanguageExtractor` trait was the natural extension point — a pluggable design without plugins. Implementing a parallel system for non-code files would duplicate the registry and the pipeline dispatch.

## Decision
Rename `LanguageExtractor` → `FileExtractor` and switch its discriminator from `Language` to a new `FileKind { Code(Language), Doc(DocFormat), Config(ConfigFormat), Table(TableFormat), Unknown }`. Add three `LeafKind` variants: `Section { depth: u8 }` (markdown heading), `ConfigKey` (top-level key in YAML/JSON/TOML), `Column` (header cell in CSV/TSV). Ship shallow extractors only: ATX headings (not frontmatter, not fenced blocks), top-level map entries (not nested paths), first-row cells (not row-level nodes). 1 MiB size cap on tabular extraction short-circuits before parsing. Extraction is the only pipeline path that changes — `FileKind::from_extension` replaces `Language::from_extension` at build time.

**Amendment ([T20260509-64]).** ADR-038 collapses the config and table extractors to file-as-leaf: YAML/JSON/TOML/CSV/TSV files keep `FileKind::Config(_)` / `FileKind::Table(_)` classification but emit zero leaves. The `ConfigKey` and `Column` `LeafKind` variants remain in the enum for forward compatibility but are no longer produced. Markdown `Section { depth }` extraction is unchanged.

## Consequences
- Markdown section anchors are first-class graph nodes; config keys and table columns are not (per ADR-038).
- Stored index-file `kind` field switches from direct enum serialization to `LeafKind::to_string()` — required because `Section { depth }` serializes as `{"section": {"depth": 1}}` and the index's `kind: Option<String>` consumer expects bare strings. The full depth payload lives in the object body.
- Cost: `LeafKind` JSON shape becomes heterogeneous (some variants are bare strings, `Section { depth }` is an externally-tagged object). Acceptable because no consumer pins the full LeafKind JSON string; `#[non_exhaustive]` not yet set on the enum — future LeafKind additions remain a breaking change for downstream exhaustive matches.

---
