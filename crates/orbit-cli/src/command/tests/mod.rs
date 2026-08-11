// Content moved from inline #[cfg(test)] mod tests in command/mod.rs per ORB-00221.
// tests/mod.rs can directly contain tests for the declaring parent module (exempt from orphan rules).

mod doctor;
mod friction;
mod gc;
mod init;
mod operation;
mod operation_args;
mod sweep;

use clap::{Parser, error::ErrorKind};

use orbit_common::types::McpCapability;

use super::{
    Cli, Commands,
    docs::DocsSubcommand,
    hook::HookSubcommand,
    mcp::McpSubcommand,
    search::SearchSubcommand,
    semantic::{SemanticIndexKindArg, SemanticSubcommand},
    web::WebSubcommand,
};

fn assert_cli_rejects(args: &[&str], kind: ErrorKind, expected: &str) {
    let error = match Cli::try_parse_from(args.iter().copied()) {
        Ok(_) => panic!("form should be rejected"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), kind, "{error}");
    let message = error.to_string();
    assert!(message.contains(expected), "{message}");
}

#[test]
fn cli_parses_doctor_stale_lock_cleanup() {
    let cli = Cli::parse_from([
        "orbit",
        "doctor",
        "--fix-stale-locks",
        "--remove-graph",
        "--json",
    ]);
    match cli.command {
        Commands::Doctor(command) => {
            assert!(command.fix_stale_locks);
            assert!(!command.fix_stale_task_locks);
            assert!(command.remove_graph);
            assert!(command.json);
            // [ORB-10501] Repairs are opt-in: an unflagged run only diagnoses.
            assert!(!command.fix_orphaned_allocations);
        }
        _ => panic!("expected top-level doctor command"),
    }
}

#[test]
fn cli_parses_doctor_stale_task_lock_repair_without_blanket_fix() {
    let cli = Cli::parse_from(["orbit", "doctor", "--fix-stale-task-locks"]);
    match cli.command {
        Commands::Doctor(command) => {
            assert!(command.fix_stale_task_locks);
            assert!(!command.fix_stale_locks);
            assert!(!command.fix_orphaned_allocations);
        }
        _ => panic!("expected top-level doctor command"),
    }

    assert_cli_rejects(
        &["orbit", "doctor", "--fix"],
        ErrorKind::UnknownArgument,
        "unexpected argument '--fix'",
    );
}

/// [ORB-10501] The guarded repair for allocations pinned to a reaped worktree.
#[test]
fn cli_parses_doctor_orphaned_allocation_repair() {
    let cli = Cli::parse_from(["orbit", "doctor", "--fix-orphaned-allocations"]);
    match cli.command {
        Commands::Doctor(command) => {
            assert!(command.fix_orphaned_allocations);
            assert!(!command.fix_stale_locks);
        }
        _ => panic!("expected top-level doctor command"),
    }
}

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
fn cli_parses_hub_mcp_serve_with_one_exact_capability() {
    let cli = Cli::parse_from([
        "orbit",
        "mcp",
        "serve",
        "--hub",
        "--capabilities",
        "operator",
    ]);
    match cli.command {
        Commands::Mcp(command) => match command.command {
            McpSubcommand::Serve(args) => {
                assert!(args.hub);
                assert_eq!(args.capabilities, Some(McpCapability::Operator));
            }
            _ => panic!("expected mcp serve"),
        },
        _ => panic!("expected top-level mcp command"),
    }
}

#[test]
fn cli_parses_broker_capability_and_rejects_unknown_values() {
    let cli = Cli::parse_from(["orbit", "mcp", "serve", "--capabilities", "operator"]);
    match cli.command {
        Commands::Mcp(command) => match command.command {
            McpSubcommand::Serve(args) => {
                assert!(!args.hub);
                assert_eq!(args.capabilities, Some(McpCapability::Operator));
            }
            _ => panic!("expected mcp serve"),
        },
        _ => panic!("expected top-level mcp command"),
    }
    assert_cli_rejects(
        &["orbit", "mcp", "serve", "--hub", "--capabilities", "admin"],
        ErrorKind::ValueValidation,
        "admin",
    );
}

#[test]
fn cli_parses_web_serve() {
    let cli = Cli::parse_from(["orbit", "web", "serve"]);
    match cli.command {
        Commands::Web(command) => match command.command {
            WebSubcommand::Serve(_) => {}
            WebSubcommand::Connect(_) => panic!("expected serve"),
        },
        _ => panic!("expected top-level web command"),
    }
}

#[test]
fn cli_parses_web_serve_global_as_deprecated_noop() {
    // `--global` is a deprecated no-op (ORB-10029): `orbit web serve` always
    // serves in global mode now, but the flag must keep parsing since `orbit
    // web connect` forwards it to remote hosts that may run an older binary.
    let cli = Cli::parse_from(["orbit", "web", "serve", "--global"]);
    match cli.command {
        Commands::Web(command) => match command.command {
            WebSubcommand::Serve(args) => assert!(args.global),
            WebSubcommand::Connect(_) => panic!("expected serve"),
        },
        _ => panic!("expected top-level web command"),
    }
}

#[test]
fn cli_parses_web_connect() {
    let cli = Cli::parse_from(["orbit", "web", "connect", "my-host", "--no-open"]);
    match cli.command {
        Commands::Web(command) => match command.command {
            WebSubcommand::Connect(args) => {
                assert_eq!(args.ssh_host, "my-host");
                assert!(args.no_open);
            }
            WebSubcommand::Serve(_) => panic!("expected connect"),
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
            _ => panic!("expected hook pretooluse"),
        },
        _ => panic!("expected top-level hook command"),
    }
}

#[test]
fn cli_parses_hook_install_and_uninstall() {
    let cli = Cli::parse_from(["orbit", "hook", "install"]);
    match cli.command {
        Commands::Hook(command) => match command.command {
            HookSubcommand::Install(_) => {}
            _ => panic!("expected hook install"),
        },
        _ => panic!("expected top-level hook command"),
    }

    let cli = Cli::parse_from(["orbit", "hook", "uninstall"]);
    match cli.command {
        Commands::Hook(command) => match command.command {
            HookSubcommand::Uninstall(_) => {}
            _ => panic!("expected hook uninstall"),
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
    for kind in ["adr", "adrs", "learning"] {
        let error = match Cli::try_parse_from(["orbit", "semantic", "index", "--kind", kind]) {
            Ok(_) => panic!("singular kinds should be rejected"),
            Err(error) => error,
        };
        let message = error.to_string();
        assert!(message.contains("possible values"), "{message}");
        assert!(message.contains("tasks"), "{message}");
        assert!(message.contains("docs"), "{message}");
        assert!(message.contains("learnings"), "{message}");
        assert!(message.contains("all"), "{message}");
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
            "--kind selects corpus: tasks (default), docs (same as `orbit docs index`), learnings, all (rebuilds all indexed corpora)."
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
    assert_cli_rejects(
        &["orbit", "docs", "reindex"],
        ErrorKind::InvalidSubcommand,
        "unrecognized subcommand 'reindex'",
    );
}

#[test]
fn cli_rejects_learning_reindex() {
    assert_cli_rejects(
        &["orbit", "learning", "reindex"],
        ErrorKind::InvalidSubcommand,
        "unrecognized subcommand 'reindex'",
    );
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
fn cli_rejects_retired_adr_search_kind() {
    assert_cli_rejects(
        &["orbit", "search", "perf", "--kind", "adr"],
        ErrorKind::InvalidValue,
        "invalid value 'adr'",
    );
}

#[test]
fn cli_rejects_search_query_with_semantic_neighbor() {
    assert_cli_rejects(
        &["orbit", "search", "query", "ORB-1"],
        ErrorKind::UnknownArgument,
        "unexpected argument 'ORB-1'",
    );
}

#[test]
fn cli_rejects_search_related_flag() {
    let legacy_flag = concat!("--", "related");
    assert_cli_rejects(
        &["orbit", "search", legacy_flag, "ORB-1"],
        ErrorKind::UnknownArgument,
        "unexpected argument '--related'",
    );
}

#[test]
fn cli_rejects_search_semantic_flag() {
    assert_cli_rejects(
        &["orbit", "search", "--semantic", "ORB-1"],
        ErrorKind::UnknownArgument,
        "unexpected argument '--semantic'",
    );
}

#[test]
fn cli_rejects_retired_search_field_and_model_flags() {
    for (args, retired_flag) in [
        (
            &["orbit", "search", "query", "--field", "title"][..],
            "--field",
        ),
        (
            &["orbit", "search", "query", "--model", "bge-small"][..],
            "--model",
        ),
        (
            &["orbit", "search", "similar", "ORB-1", "--field", "title"][..],
            "--field",
        ),
        (
            &["orbit", "search", "path", "crates/", "--model", "bge-small"][..],
            "--model",
        ),
    ] {
        assert_cli_rejects(
            args,
            ErrorKind::UnknownArgument,
            &format!("unexpected argument '{retired_flag}'"),
        );
    }
}

#[test]
fn cli_rejects_retired_search_path_flag() {
    assert_cli_rejects(
        &["orbit", "search", "--path", "crates/"],
        ErrorKind::UnknownArgument,
        "unexpected argument '--path'",
    );
}

#[test]
fn cli_rejects_top_level_serve() {
    assert_cli_rejects(
        &["orbit", "serve"],
        ErrorKind::InvalidSubcommand,
        "unrecognized subcommand 'serve'",
    );
}

#[test]
fn cli_rejects_down_alias() {
    assert_cli_rejects(
        &["orbit", "mcp", "down"],
        ErrorKind::InvalidSubcommand,
        "unrecognized subcommand 'down'",
    );
}
