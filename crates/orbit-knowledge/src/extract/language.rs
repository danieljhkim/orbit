//! File classification for the extractor dispatch.
//!
//! `FileKind` is the canonical classifier across the knowledge graph. Code
//! languages live under `FileKind::Code(Language)` to keep the existing
//! per-language extractor dispatch intact; docs, configs, and tabular data
//! each live under their own discriminator.
//!
//! Added in T20260422-1540 (non-code extraction). The former `Language`-based
//! dispatch is retained as a sub-variant for compatibility with tree-sitter
//! extractors; no call site changed shape.

/// Supported source-code languages with tree-sitter extractors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    C,
    CSharp,
    Rust,
    Python,
    Go,
    Java,
    JavaScript,
    Kotlin,
    TypeScript,
    Tsx,
    Ruby,
}

impl Language {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "c" | "h" => Some(Self::C),
            "cs" => Some(Self::CSharp),
            "rs" => Some(Self::Rust),
            "py" => Some(Self::Python),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "kt" | "kts" => Some(Self::Kotlin),
            "ts" | "mts" | "cts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            // Ruby tooling commonly uses .rake tasks and .gemspec manifests;
            // both are plain Ruby syntax for extractor purposes.
            "rb" | "rake" | "gemspec" => Some(Self::Ruby),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::C => "c",
            Self::CSharp => "csharp",
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Go => "go",
            Self::Java => "java",
            Self::JavaScript => "javascript",
            Self::Kotlin => "kotlin",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::Ruby => "ruby",
        }
    }
}

/// Documentation format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocFormat {
    Markdown,
}

/// Structured configuration format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Yaml,
    Json,
    Toml,
}

/// Tabular data format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableFormat {
    Csv,
    Tsv,
}

/// Classification of a file for extractor dispatch.
///
/// Dispatch order: first by outer variant (`Code`/`Doc`/`Config`/`Table`),
/// then by sub-format within each family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Code(Language),
    Doc(DocFormat),
    Config(ConfigFormat),
    Table(TableFormat),
    Unknown,
}

impl FileKind {
    pub fn from_extension(ext: &str) -> Self {
        if let Some(lang) = Language::from_extension(ext) {
            return Self::Code(lang);
        }
        match ext {
            "md" => Self::Doc(DocFormat::Markdown),
            "yaml" | "yml" => Self::Config(ConfigFormat::Yaml),
            "json" => Self::Config(ConfigFormat::Json),
            "toml" => Self::Config(ConfigFormat::Toml),
            "csv" => Self::Table(TableFormat::Csv),
            "tsv" => Self::Table(TableFormat::Tsv),
            _ => Self::Unknown,
        }
    }

    /// Short identifier used for the `language` field on leaf/file nodes.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Code(lang) => lang.as_str(),
            Self::Doc(DocFormat::Markdown) => "markdown",
            Self::Config(ConfigFormat::Yaml) => "yaml",
            Self::Config(ConfigFormat::Json) => "json",
            Self::Config(ConfigFormat::Toml) => "toml",
            Self::Table(TableFormat::Csv) => "csv",
            Self::Table(TableFormat::Tsv) => "tsv",
            Self::Unknown => "",
        }
    }

    pub fn is_extractable(&self) -> bool {
        !matches!(self, Self::Unknown)
    }
}

#[cfg(test)]
mod tests;
