//! Raw graph row types produced by extractors before storage.

/// Extracted graph rows for a single file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedFile {
    /// Symbols defined in the file.
    pub symbols: Vec<RawSymbol>,
    /// Textual references from source spans to symbol names.
    pub refs: Vec<RawRef>,
    /// Symbol-to-symbol structural edges.
    pub relations: Vec<RawRelation>,
    /// Module-level imports or use statements.
    pub imports: Vec<RawImport>,
    /// Notable string literals and document text snippets.
    pub strings: Vec<RawString>,
    /// Structured configuration keys.
    pub configs: Vec<RawConfig>,
    /// CLI command surface entries.
    pub commands: Vec<RawCommand>,
}

/// Raw row for a symbol defined in a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawSymbol {
    /// Path of the file containing the symbol.
    pub file_path: String,
    /// Short symbol name.
    pub name: String,
    /// Stable qualified symbol name.
    pub qualified: String,
    /// Symbol kind, such as `function`, `struct`, `method`, or `module`.
    pub kind: String,
    /// Start byte offset into the source file.
    pub span_start: usize,
    /// Exclusive end byte offset into the source file.
    pub span_end: usize,
    /// One-line normalized signature when available.
    pub signature: Option<String>,
    /// Qualified parent symbol when the extractor can identify one.
    pub parent_symbol: Option<String>,
}

/// Raw row for a textual reference from a source span to a symbol name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawRef {
    /// Path of the file containing the reference.
    pub from_file: String,
    /// Start byte offset of the reference span.
    pub from_span_start: usize,
    /// Exclusive end byte offset of the reference span.
    pub from_span_end: usize,
    /// Short target name found at the span.
    pub target_name: String,
    /// Best-effort qualified target name when known at extraction time.
    pub target_qualified: Option<String>,
    /// Reference kind, such as `call`, `type`, `use`, or `trait_bound`.
    pub kind: String,
    /// Confidence label from the graph confidence ladder.
    pub confidence: String,
}

/// Raw row for a structural relationship between two symbols.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawRelation {
    /// Qualified source symbol name.
    pub from_qualified: String,
    /// Qualified target symbol name.
    pub to_qualified: String,
    /// Relation kind, such as `impl`, `extends`, or `implements`.
    pub kind: String,
    /// Path of the file containing the relation definition.
    pub def_file: String,
    /// Start byte offset of the relation definition.
    pub def_span_start: usize,
    /// Exclusive end byte offset of the relation definition.
    pub def_span_end: usize,
    /// Confidence label from the graph confidence ladder.
    pub confidence: String,
}

/// Raw row for an import or use statement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawImport {
    /// Path of the file containing the import.
    pub from_file: String,
    /// Language-specific opaque target path.
    pub target_path: String,
    /// Imported symbol name, or `None` for whole-module imports.
    pub target_symbol: Option<String>,
}

/// Raw row for a notable string literal or document snippet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawString {
    /// Path of the file containing the string.
    pub file_path: String,
    /// One-based source line containing the string.
    pub line: usize,
    /// String value retained for graph search.
    pub value: String,
    /// Qualified context symbol when the extractor can identify one.
    pub context_symbol: Option<String>,
}

/// Raw row for a structured configuration key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawConfig {
    /// Path of the file containing the configuration key.
    pub file_path: String,
    /// One-based source line containing the key.
    pub line: usize,
    /// Configuration key.
    pub key: String,
    /// Config kind, such as `yaml`, `toml`, `json`, `env`, or `serde`.
    pub kind: String,
}

/// Raw row for a CLI command surface entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawCommand {
    /// Command name.
    pub name: String,
    /// Path of the file containing the command definition.
    pub file_path: String,
    /// Start byte offset of the command definition.
    pub span_start: usize,
    /// Qualified handler symbol when the extractor can identify one.
    pub handler_symbol: Option<String>,
}
