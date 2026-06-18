mod matching {
    use super::super::super::glob::*;
    use crate::types::OrbitError;

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
