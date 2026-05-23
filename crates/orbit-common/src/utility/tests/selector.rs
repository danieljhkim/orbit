mod matching {
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::super::super::selector::*;

    #[test]
    fn anchor_path_extracts_symbol_file_path() {
        assert_eq!(
            anchor_path("symbol:src/lib.rs#run:function").unwrap(),
            PathBuf::from("src/lib.rs")
        );
    }

    #[test]
    fn exists_in_workspace_uses_anchor_paths() {
        let temp = tempdir().unwrap();
        let workspace = temp.path();
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::write(workspace.join("src/lib.rs"), "pub fn ok() {}\n").unwrap();

        assert!(exists_in_workspace(
            "symbol:src/lib.rs#run:function",
            workspace
        ));
        assert!(!exists_in_workspace(
            "symbol:src/missing.rs#run:function",
            workspace
        ));
    }

    #[test]
    fn overlaps_uses_anchor_semantics() {
        assert!(overlaps("symbol:f.rs#a:method", "symbol:f.rs#b:method"));
        assert!(overlaps("dir:src", "file:src/lib.rs"));
        assert!(overlaps("src", "file:src/lib.rs"));
        assert!(!overlaps("file:f.rs", "file:g.rs"));
        assert!(!overlaps("dir:src", "file:lib/y.rs"));
    }

    #[test]
    fn shared_anchor_prefix_depth_ignores_selector_metadata() {
        assert_eq!(
            shared_anchor_prefix_depth(
                "symbol:src/lib.rs#alpha:function",
                "file:src/nested/mod.rs"
            ),
            1
        );
    }
}

mod parse {
    use std::path::PathBuf;
    use std::str::FromStr;

    use proptest::prelude::*;
    use proptest::test_runner::Config as ProptestConfig;
    use tempfile::tempdir;

    use super::super::super::selector::*;

    fn path_segment() -> impl Strategy<Value = String> {
        prop::string::string_regex("[a-z][a-z0-9_]{0,8}").expect("valid path segment regex")
    }

    fn selector_path() -> impl Strategy<Value = String> {
        prop::collection::vec(path_segment(), 1..5).prop_map(|segments| segments.join("/"))
    }

    fn identifier() -> impl Strategy<Value = String> {
        prop::string::string_regex("[A-Za-z_][A-Za-z0-9_]{0,12}").expect("valid identifier regex")
    }

    fn symbol_name() -> impl Strategy<Value = String> {
        prop_oneof![
            identifier(),
            (identifier(), identifier()).prop_map(|(module, name)| format!("{module}::{name}")),
            (identifier(), identifier(), identifier())
                .prop_map(|(ty, trait_name, method)| format!("<{ty} as {trait_name}>::{method}")),
        ]
    }

    fn kind_name() -> impl Strategy<Value = String> {
        prop::string::string_regex("[a-z][a-z_]{0,12}").expect("valid kind regex")
    }

    fn dir_selector() -> impl Strategy<Value = Selector> {
        selector_path().prop_map(|path| Selector::Dir { path })
    }

    fn file_selector() -> impl Strategy<Value = Selector> {
        selector_path().prop_map(|path| Selector::File { path })
    }

    fn symbol_selector() -> impl Strategy<Value = Selector> {
        (selector_path(), symbol_name(), kind_name())
            .prop_map(|(path, symbol, kind)| Selector::Symbol { path, symbol, kind })
    }

    #[test]
    fn canonical_selector_handles_raw_paths_and_ranges() {
        assert_eq!(canonical_selector("src/lib.rs").unwrap(), "file:src/lib.rs");
        assert_eq!(
            canonical_selector("src/lib.rs:42").unwrap(),
            "file:src/lib.rs"
        );
        assert_eq!(
            canonical_selector("src/lib.rs:42:7").unwrap(),
            "file:src/lib.rs"
        );
        assert_eq!(
            canonical_selector("src/mod.rs:10-20").unwrap(),
            "file:src/mod.rs"
        );
        assert_eq!(canonical_selector("src/").unwrap(), "dir:src");
    }

    #[test]
    fn canonical_selector_in_workspace_rewrites_absolute_and_directory_paths() {
        let temp = tempdir().unwrap();
        let workspace = temp.path();
        std::fs::create_dir_all(workspace.join("src/nested")).unwrap();
        std::fs::write(workspace.join("src/lib.rs"), "pub fn ok() {}\n").unwrap();

        assert_eq!(
            canonical_selector_in_workspace(
                &workspace.join("src/lib.rs").to_string_lossy(),
                workspace
            )
            .unwrap(),
            "file:src/lib.rs"
        );
        assert_eq!(
            canonical_selector_in_workspace("src/nested", workspace).unwrap(),
            "dir:src/nested"
        );
    }

    #[test]
    fn symbol_selector_preserves_opaque_qualified_name() {
        let selector: Selector = "symbol:src/lib.rs#<Foo as Runnable>::run#2:method"
            .parse()
            .unwrap();

        assert_eq!(
            selector,
            Selector::Symbol {
                path: "src/lib.rs".to_string(),
                symbol: "<Foo as Runnable>::run#2".to_string(),
                kind: "method".to_string(),
            }
        );
        assert_eq!(
            selector.to_string(),
            "symbol:src/lib.rs#<Foo as Runnable>::run#2:method"
        );
        assert_eq!(
            anchor_path(&selector.to_string()).unwrap(),
            PathBuf::from("src/lib.rs")
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, .. ProptestConfig::default() })]

        #[test]
        fn dir_selector_display_parse_roundtrips(selector in dir_selector()) {
            prop_assert_eq!(Selector::from_str(&selector.to_string()).unwrap(), selector);
        }

        #[test]
        fn file_selector_display_parse_roundtrips(selector in file_selector()) {
            prop_assert_eq!(Selector::from_str(&selector.to_string()).unwrap(), selector);
        }

        #[test]
        fn symbol_selector_display_parse_roundtrips(selector in symbol_selector()) {
            prop_assert_eq!(Selector::from_str(&selector.to_string()).unwrap(), selector);
        }
    }
}
