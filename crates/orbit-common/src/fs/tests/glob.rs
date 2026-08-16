mod matching {
    use crate::OrbitError;
    use crate::fs::glob::*;

    #[test]
    fn double_star_matches_nested_paths() {
        let path = normalize_glob_path("crates/orbit-engine/perf_runner.rs").expect("normalize");
        assert!(match_glob("**/perf*.rs", &path).expect("match glob"));
    }

    #[test]
    fn double_star_rejects_non_matching_filename() {
        let path = normalize_glob_path("crates/orbit-engine/runner.rs").expect("normalize");
        assert!(!match_glob("**/perf*.rs", &path).expect("match glob"));
    }

    #[test]
    fn normalize_strips_leading_dot_slash_and_backslashes() {
        let path = normalize_glob_path("./crates\\orbit-engine/perf.rs").expect("normalize");
        assert_eq!(path, "crates/orbit-engine/perf.rs");
    }

    /// [ORB-10009] Deny-bypass regression: `secret/./key.txt`,
    /// `secret//key.txt`, and `secret/key.txt/` all name the same file as
    /// `secret/key.txt`; normalization must collapse them so an exact deny
    /// rule cannot be dodged by respelling the path.
    #[test]
    fn normalize_collapses_dot_segments_duplicate_and_trailing_separators() {
        for spelling in [
            "secret/./key.txt",
            "secret//key.txt",
            "secret/key.txt/",
            "././secret/key.txt",
            "secret/.//./key.txt",
        ] {
            let normalized = normalize_glob_path(spelling).expect("normalize");
            assert_eq!(
                normalized, "secret/key.txt",
                "`{spelling}` must normalize to the canonical spelling"
            );
            assert!(
                match_glob("secret/key.txt", &normalized).expect("match"),
                "exact rule must match respelled path `{spelling}`"
            );
        }
    }

    #[test]
    fn normalize_returns_empty_string_for_workspace_root() {
        assert_eq!(normalize_glob_path(".").expect("normalize"), "");
        assert_eq!(normalize_glob_path("./").expect("normalize"), "");
    }

    #[test]
    fn normalize_rejects_traversal() {
        assert!(matches!(
            normalize_glob_path("../escape"),
            Err(OrbitError::InvalidInput(_))
        ));
    }

    #[test]
    fn trailing_double_star_matches_subtree_and_anchor() {
        let path = normalize_glob_path("foo/bar/baz.rs").expect("normalize");
        assert!(match_glob("foo/**", &path).expect("match"));

        let exact = normalize_glob_path("foo").expect("normalize");
        assert!(match_glob("foo/**", &exact).expect("match"));
    }

    #[test]
    fn single_star_does_not_cross_separator() {
        let path = normalize_glob_path("foo/bar/baz.rs").expect("normalize");
        assert!(!match_glob("foo/*.rs", &path).expect("match"));
    }

    #[test]
    fn dotenv_variant_patterns_match_prefix_and_suffix_forms() {
        for (rule, path) in [
            ("**/.env", ".env"),
            ("**/.env.*", ".env.local"),
            ("**/.env.*", "foo/.env.production"),
            ("**/*.env.*", "foo/secrets.env.bak"),
        ] {
            let path = normalize_glob_path(path).expect("normalize");
            assert!(
                match_glob(rule, &path).expect("match"),
                "rule `{rule}` should match `{path}`"
            );
        }
    }

    /// [ORB-10224] H1 regression: a `/**`-suffix rule whose prefix contains
    /// glob metacharacters (`**`, `*`) must route through the same
    /// segment-aware translation as the general path. Previously the prefix
    /// was passed through `regex::escape()`, so `**/secrets/**` compiled to
    /// `^\*\*/secrets(?:/.*)?$` and matched nothing — silently voiding the
    /// deny rule.
    #[test]
    fn wildcard_prefix_subtree_rules_match_intended_paths() {
        for (rule, path) in [
            // Subtree-at-any-depth denies — the most natural spelling.
            ("**/secrets/**", "secrets/key.txt"),
            ("**/secrets/**", "app/config/secrets/key.txt"),
            ("**/.git/**", ".git/config"),
            ("**/.git/**", "vendor/dep/.git/HEAD"),
            ("**/node_modules/**", "node_modules/left-pad/index.js"),
            (
                "**/node_modules/**",
                "packages/ui/node_modules/react/index.js",
            ),
            // Single-star prefix crosses exactly one segment.
            ("*/dir/**", "top/dir/file.rs"),
            ("*/dir/**", "top/dir/nested/file.rs"),
            // The subtree rule also anchors the directory itself.
            ("**/secrets/**", "secrets"),
            ("**/.git/**", "vendor/dep/.git"),
        ] {
            let normalized = normalize_glob_path(path).expect("normalize");
            assert!(
                match_glob(rule, &normalized).expect("match glob"),
                "rule `{rule}` should match `{normalized}`"
            );
        }
    }

    /// [ORB-10224] The wildcard-prefix subtree rule must still respect
    /// segment boundaries: a single-star prefix does not cross a separator,
    /// and a subtree deny does not leak onto a sibling that merely shares a
    /// name prefix.
    #[test]
    fn wildcard_prefix_subtree_rules_reject_non_matching_paths() {
        for (rule, path) in [
            // `*` spans one segment, so a two-segment prefix does not match.
            ("*/dir/**", "a/b/dir/file.rs"),
            // `secrets` as a path fragment, not a full segment, must not match.
            ("**/secrets/**", "my-secrets-backup/key.txt"),
            ("**/.git/**", "not-a.git/file"),
        ] {
            let normalized = normalize_glob_path(path).expect("normalize");
            assert!(
                !match_glob(rule, &normalized).expect("match glob"),
                "rule `{rule}` must not match `{normalized}`"
            );
        }
    }

    /// [ORB-10224] `validate()` behavior for metachar-in-prefix: such a rule
    /// is a *valid* glob (it compiles) and actively blocks its target, rather
    /// than being an inert, silently-passing deny.
    #[test]
    fn wildcard_prefix_subtree_rule_compiles_and_blocks() {
        let regex = compile_glob_regex("**/secrets/**").expect("compile valid glob");
        let path = normalize_glob_path("app/secrets/private.key").expect("normalize");
        assert!(
            regex.is_match(&path),
            "a `**/dir/**` deny rule must actually block its target path"
        );
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn deny_globs_match_case_variants_on_case_insensitive_platforms() {
        for (rule, path) in [
            (".orbit/**", ".Orbit/state/task.json"),
            ("**/*.env", "Secret.ENV"),
            ("**/*.env", "config/Secret.ENV"),
        ] {
            let path = normalize_glob_path(path).expect("normalize");
            assert!(
                match_glob(rule, &path).expect("match"),
                "rule `{rule}` should match case variant `{path}`"
            );
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    #[test]
    fn globs_remain_case_sensitive_on_case_sensitive_platforms() {
        let path = normalize_glob_path("Secret.ENV").expect("normalize");
        assert!(
            !match_glob("**/*.env", &path).expect("match"),
            "case-sensitive platforms should preserve distinct path identities"
        );
    }
}
