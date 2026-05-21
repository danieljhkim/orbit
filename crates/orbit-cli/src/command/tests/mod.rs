// Content moved from inline #[cfg(test)] mod tests in command/mod.rs per ORB-00221.
// tests/mod.rs can directly contain tests for the declaring parent module (exempt from orphan rules).

use clap::Parser;

use super::{
    Cli, Commands,
    docs::DocsSubcommand,
    hook::HookSubcommand,
    mcp::McpSubcommand,
    search::{SearchKindArg, SearchSubcommand},
    semantic::{SemanticIndexKindArg, SemanticSubcommand},
    web::WebSubcommand,
};

#[test]
fn cli_parses_mcp_init() {
    let cli = Cli::parse_from(["orbit", "mcp", "init"]);
    match cli.command {
        Commands::Mcp(command) => match command.command {
            McpSubcommand::Init(_) => {}
            _ => panic!("expected mcp init"),
        },
        _ => panic!("expected top-level mcp command"),
    }
}

#[test]
fn cli_parses_mcp_serve() {
    let cli = Cli::parse_from(["orbit", "mcp", "serve"]);
    match cli.command {
        Commands::Mcp(command) => match command.command {
            McpSubcommand::Serve(_) => {}
            _ => panic!("expected mcp serve"),
        },
        _ => panic!("expected top-level mcp command"),
    }
}

#[test]
fn cli_parses_web_serve() {
    let cli = Cli::parse_from(["orbit", "web", "serve"]);
    match cli.command {
        Commands::Web(command) => match command.command {
            WebSubcommand::Serve(_) => {}
        },
        _ => panic!("expected top-level web command"),
    }
}

#[test]
fn cli_parses_hook_pretooluse() {
    let cli = Cli::parse_from(["orbit", "hook", "pretooluse", "--format", "codex"]);
    match cli.command {
        Commands::Hook(command) => match command.command {
            HookSubcommand::Pretooluse(_) => {}
        },
        _ => panic!("expected top-level hook command"),
    }
}

#[test]
fn cli_parses_semantic_install_force() {
    let cli = Cli::parse_from(["orbit", "semantic", "install", "--force"]);
    match cli.command {
        Commands::Semantic(command) => match command.command {
            SemanticSubcommand::Install(args) => assert!(args.force),
            _ => panic!("expected semantic install"),
        },
        _ => panic!("expected top-level semantic command"),
    }
}

#[test]
fn cli_parses_semantic_stats() {
    let cli = Cli::parse_from(["orbit", "semantic", "stats"]);
    match cli.command {
        Commands::Semantic(command) => match command.command {
            SemanticSubcommand::Stats(_) => {}
            _ => panic!("expected semantic stats"),
        },
        _ => panic!("expected top-level semantic command"),
    }
}

#[test]
fn cli_parses_semantic_index() {
    let cli = Cli::parse_from(["orbit", "semantic", "index", "--force", "--kind", "docs"]);
    match cli.command {
        Commands::Semantic(command) => match command.command {
            SemanticSubcommand::Index(args) => {
                assert!(args.force);
                assert_eq!(args.kind, SemanticIndexKindArg::Docs);
            }
            _ => panic!("expected semantic index"),
        },
        _ => panic!("expected top-level semantic command"),
    }
}

#[test]
fn cli_semantic_index_defaults_kind_to_tasks() {
    let cli = Cli::parse_from(["orbit", "semantic", "index"]);
    match cli.command {
        Commands::Semantic(command) => match command.command {
            SemanticSubcommand::Index(args) => {
                assert_eq!(args.kind, SemanticIndexKindArg::Tasks);
            }
            _ => panic!("expected semantic index"),
        },
        _ => panic!("expected top-level semantic command"),
    }
}

#[test]
fn cli_semantic_index_rejects_singular_kinds_at_clap_layer() {
    for kind in ["adr", "learning"] {
        let error = match Cli::try_parse_from(["orbit", "semantic", "index", "--kind", kind]) {
            Ok(_) => panic!("singular kinds should be rejected"),
            Err(error) => error,
        };
        let message = error.to_string();
        assert!(message.contains("possible values"), "{message}");
        assert!(message.contains("tasks"), "{message}");
        assert!(message.contains("docs"), "{message}");
        assert!(message.contains("adrs"), "{message}");
        assert!(message.contains("learnings"), "{message}");
        assert!(message.contains("all"), "{message}");
    }
}

#[test]
fn cli_semantic_index_parses_adrs_kind() {
    let cli = Cli::parse_from(["orbit", "semantic", "index", "--kind", "adrs"]);
    match cli.command {
        Commands::Semantic(command) => match command.command {
            SemanticSubcommand::Index(args) => {
                assert_eq!(args.kind, SemanticIndexKindArg::Adrs);
            }
            _ => panic!("expected semantic index"),
        },
        _ => panic!("expected top-level semantic command"),
    }
}

#[test]
fn cli_semantic_index_parses_learnings_kind() {
    let cli = Cli::parse_from(["orbit", "semantic", "index", "--kind", "learnings"]);
    match cli.command {
        Commands::Semantic(command) => match command.command {
            SemanticSubcommand::Index(args) => {
                assert_eq!(args.kind, SemanticIndexKindArg::Learnings);
            }
            _ => panic!("expected semantic index"),
        },
        _ => panic!("expected top-level semantic command"),
    }
}

#[test]
fn cli_semantic_index_help_explains_kind_principle() {
    let error = match Cli::try_parse_from(["orbit", "semantic", "index", "--help"]) {
        Ok(_) => panic!("help exits before parsing"),
        Err(error) => error,
    };
    let help = error.to_string();
    assert!(
        help.contains(
            "--kind selects corpus: tasks (default), docs (same as `orbit docs index`), adrs, learnings, all (rebuilds all indexed corpora)."
        ),
        "{help}"
    );
}

#[test]
fn cli_parses_docs_index() {
    let cli = Cli::parse_from(["orbit", "docs", "index", "--force", "--model", "minilm-l6"]);
    match cli.command {
        Commands::Docs(command) => match command.command {
            DocsSubcommand::Index(args) => {
                assert!(args.force);
                assert_eq!(args.model.as_deref(), Some("minilm-l6"));
            }
            _ => panic!("expected docs index"),
        },
        _ => panic!("expected top-level docs command"),
    }
}

#[test]
fn cli_rejects_docs_reindex() {
    assert!(Cli::try_parse_from(["orbit", "docs", "reindex"]).is_err());
}

#[test]
fn cli_parses_top_level_search() {
    let cli = Cli::parse_from([
        "orbit",
        "search",
        "semantic search design",
        "--hybrid",
        "--kind",
        "task",
    ]);
    match cli.command {
        Commands::Search(args) => {
            assert_eq!(args.query.as_deref(), Some("semantic search design"));
            assert!(args.hybrid);
            assert!(args.command.is_none());
        }
        _ => panic!("expected top-level search command"),
    }
}

#[test]
fn cli_parses_top_level_search_similar_neighbor() {
    let cli = Cli::parse_from(["orbit", "search", "similar", "ORB-1"]);
    match cli.command {
        Commands::Search(args) => {
            assert_eq!(args.query, None);
            match args.command {
                Some(SearchSubcommand::Similar(similar)) => {
                    assert_eq!(similar.id, "ORB-1");
                }
                _ => panic!("expected search similar"),
            }
        }
        _ => panic!("expected top-level search command"),
    }
}

#[test]
fn cli_parses_top_level_search_path_lookup() {
    let cli = Cli::parse_from(["orbit", "search", "path", "crates/orbit-cli/"]);
    match cli.command {
        Commands::Search(args) => {
            assert_eq!(args.query, None);
            match args.command {
                Some(SearchSubcommand::Path(path)) => {
                    assert_eq!(path.path, "crates/orbit-cli/");
                }
                _ => panic!("expected search path"),
            }
        }
        _ => panic!("expected top-level search command"),
    }
}

#[test]
fn cli_parses_top_level_search_tag_filter() {
    let cli = Cli::parse_from(["orbit", "search", "perf", "--tag", "perf", "--kind", "adr"]);
    match cli.command {
        Commands::Search(args) => {
            assert_eq!(args.query.as_deref(), Some("perf"));
            assert_eq!(args.tags, vec!["perf"]);
            assert_eq!(args.kind, SearchKindArg::Adr);
        }
        _ => panic!("expected top-level search command"),
    }
}

#[test]
fn cli_rejects_search_query_with_semantic_neighbor() {
    assert!(Cli::try_parse_from(["orbit", "search", "query", "ORB-1"]).is_err());
}

#[test]
fn cli_rejects_search_related_flag() {
    let legacy_flag = concat!("--", "related");
    assert!(Cli::try_parse_from(["orbit", "search", legacy_flag, "ORB-1"]).is_err());
}

#[test]
fn cli_rejects_search_semantic_flag() {
    assert!(Cli::try_parse_from(["orbit", "search", "--semantic", "ORB-1"]).is_err());
}

#[test]
fn cli_rejects_retired_search_field_and_model_flags() {
    assert!(Cli::try_parse_from(["orbit", "search", "query", "--field", "title"]).is_err());
    assert!(Cli::try_parse_from(["orbit", "search", "query", "--model", "bge-small"]).is_err());
    assert!(
        Cli::try_parse_from(["orbit", "search", "similar", "ORB-1", "--field", "title"]).is_err()
    );
    assert!(
        Cli::try_parse_from(["orbit", "search", "path", "crates/", "--model", "bge-small"])
            .is_err()
    );
}

#[test]
fn cli_rejects_retired_search_path_flag() {
    assert!(Cli::try_parse_from(["orbit", "search", "--path", "crates/"]).is_err());
}

#[test]
fn cli_rejects_top_level_serve() {
    assert!(Cli::try_parse_from(["orbit", "serve"]).is_err());
}

#[test]
fn cli_rejects_down_alias() {
    assert!(Cli::try_parse_from(["orbit", "mcp", "down"]).is_err());
}
