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
    Rust,
    Python,
    Go,
    Java,
    JavaScript,
    TypeScript,
    Tsx,
    Ruby,
}

impl Language {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "c" | "h" => Some(Self::C),
            "rs" => Some(Self::Rust),
            "py" => Some(Self::Python),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
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
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Go => "go",
            Self::Java => "java",
            Self::JavaScript => "javascript",
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
mod tests {
    use super::*;

    #[test]
    fn from_extension_classifies_code() {
        assert_eq!(FileKind::from_extension("c"), FileKind::Code(Language::C));
        assert_eq!(FileKind::from_extension("h"), FileKind::Code(Language::C));
        assert_eq!(
            FileKind::from_extension("rs"),
            FileKind::Code(Language::Rust)
        );
        assert_eq!(
            FileKind::from_extension("py"),
            FileKind::Code(Language::Python)
        );
        assert_eq!(FileKind::from_extension("go"), FileKind::Code(Language::Go));
        assert_eq!(
            FileKind::from_extension("java"),
            FileKind::Code(Language::Java)
        );
        assert_eq!(
            FileKind::from_extension("js"),
            FileKind::Code(Language::JavaScript)
        );
        assert_eq!(
            FileKind::from_extension("jsx"),
            FileKind::Code(Language::JavaScript)
        );
        assert_eq!(
            FileKind::from_extension("mjs"),
            FileKind::Code(Language::JavaScript)
        );
        assert_eq!(
            FileKind::from_extension("cjs"),
            FileKind::Code(Language::JavaScript)
        );
        assert_eq!(
            FileKind::from_extension("ts"),
            FileKind::Code(Language::TypeScript)
        );
        assert_eq!(
            FileKind::from_extension("mts"),
            FileKind::Code(Language::TypeScript)
        );
        assert_eq!(
            FileKind::from_extension("cts"),
            FileKind::Code(Language::TypeScript)
        );
        assert_eq!(
            FileKind::from_extension("tsx"),
            FileKind::Code(Language::Tsx)
        );
        assert_eq!(
            FileKind::from_extension("rb"),
            FileKind::Code(Language::Ruby)
        );
        assert_eq!(
            FileKind::from_extension("rake"),
            FileKind::Code(Language::Ruby)
        );
        assert_eq!(
            FileKind::from_extension("gemspec"),
            FileKind::Code(Language::Ruby)
        );
        assert_eq!(FileKind::from_extension("ts").as_str(), "typescript");
        assert_eq!(FileKind::from_extension("tsx").as_str(), "tsx");
        assert_eq!(FileKind::from_extension("h").as_str(), "c");
        assert_eq!(FileKind::from_extension("rb").as_str(), "ruby");
    }

    #[test]
    fn from_extension_classifies_docs() {
        assert_eq!(
            FileKind::from_extension("md"),
            FileKind::Doc(DocFormat::Markdown)
        );
    }

    #[test]
    fn from_extension_classifies_config() {
        assert_eq!(
            FileKind::from_extension("yaml"),
            FileKind::Config(ConfigFormat::Yaml)
        );
        assert_eq!(
            FileKind::from_extension("yml"),
            FileKind::Config(ConfigFormat::Yaml)
        );
        assert_eq!(
            FileKind::from_extension("json"),
            FileKind::Config(ConfigFormat::Json)
        );
        assert_eq!(
            FileKind::from_extension("toml"),
            FileKind::Config(ConfigFormat::Toml)
        );
    }

    #[test]
    fn from_extension_classifies_tables() {
        assert_eq!(
            FileKind::from_extension("csv"),
            FileKind::Table(TableFormat::Csv)
        );
        assert_eq!(
            FileKind::from_extension("tsv"),
            FileKind::Table(TableFormat::Tsv)
        );
    }

    #[test]
    fn from_extension_returns_unknown_for_unrecognized() {
        assert_eq!(FileKind::from_extension("xyz"), FileKind::Unknown);
        assert_eq!(FileKind::from_extension(""), FileKind::Unknown);
    }

    #[test]
    fn is_extractable_gates_unknown() {
        assert!(FileKind::from_extension("md").is_extractable());
        assert!(!FileKind::from_extension("xyz").is_extractable());
    }
}
