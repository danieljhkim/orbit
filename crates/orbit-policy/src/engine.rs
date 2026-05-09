use orbit_common::types::{FsOperation, OrbitError, PolicyDef};

use crate::evaluator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsPolicyEvaluation {
    pub profile: String,
    pub operation: FsOperation,
    pub path: String,
    pub allowed: bool,
    pub matched_rule: String,
}

#[derive(Debug, Clone)]
pub struct PolicyEngine {
    def: PolicyDef,
}

impl PolicyEngine {
    pub fn from_def(def: &PolicyDef) -> Result<Self, OrbitError> {
        def.validate()?;
        Ok(Self { def: def.clone() })
    }

    pub fn check(
        &self,
        profile: impl Into<String>,
        operation: FsOperation,
        path: impl Into<String>,
    ) -> Result<FsPolicyEvaluation, OrbitError> {
        let profile = profile.into();
        let path = path.into();
        let result = evaluator::evaluate(&self.def, &profile, operation, &path)?;
        Ok(FsPolicyEvaluation {
            profile,
            operation,
            path,
            allowed: result.allowed,
            matched_rule: result.matched_rule,
        })
    }

    pub fn def(&self) -> &PolicyDef {
        &self.def
    }
}

#[cfg(test)]
mod tests {
    //! Boundary tests for `PolicyEngine::check`. These guard the global
    //! `denyRead` / `denyModify` last-match-wins semantics, the unknown-profile
    //! error path, and matched_rule observability for audit attribution.
    //! See task T20260509-7.

    use super::*;
    use chrono::Utc;
    use orbit_common::types::policy_def::FsProfile;
    use std::collections::HashMap;

    fn make_def(
        deny_read: Vec<&str>,
        deny_modify: Vec<&str>,
        profiles: &[(&str, &[&str], &[&str])],
    ) -> PolicyDef {
        let mut fs_profiles = HashMap::new();
        for (name, read, modify) in profiles {
            fs_profiles.insert(
                (*name).to_string(),
                FsProfile {
                    read: read.iter().map(|s| (*s).to_string()).collect(),
                    modify: modify.iter().map(|s| (*s).to_string()).collect(),
                },
            );
        }
        PolicyDef {
            name: "test".to_string(),
            description: None,
            deny_read: deny_read.into_iter().map(String::from).collect(),
            deny_modify: deny_modify.into_iter().map(String::from).collect(),
            fs_profiles,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn check_returns_allowed_when_path_inside_profile_read_rule() {
        // Invariant: a path matching a positive `read` rule resolves to
        // allowed=true with the matching rule recorded.
        let def = make_def(vec![], vec![], &[("default", &["src/**"], &["src/**"])]);
        let engine = PolicyEngine::from_def(&def).expect("engine");

        let result = engine
            .check("default", FsOperation::Read, "src/foo.rs")
            .expect("check");

        assert!(result.allowed);
        assert_eq!(result.matched_rule, "src/**");
    }

    #[test]
    fn check_returns_denied_when_path_outside_modify_rules() {
        // Invariant: a Modify path that no positive rule matches resolves to
        // allowed=false. The matched_rule reflects the empty/no-match outcome
        // so the audit trail can attribute the deny.
        let def = make_def(vec![], vec![], &[("default", &["src/**"], &["src/**"])]);
        let engine = PolicyEngine::from_def(&def).expect("engine");

        let result = engine
            .check("default", FsOperation::Modify, "tests/foo.rs")
            .expect("check");

        assert!(!result.allowed);
        assert!(
            !result.matched_rule.is_empty(),
            "matched_rule must record the deny reason for audit attribution"
        );
    }

    #[test]
    fn check_global_deny_modify_overrides_profile_modify_allow() {
        // Invariant (CLAUDE.md "global denyModify rules accumulate"): a
        // global `denyModify` rule must beat a profile-level positive
        // `modify` rule under last-match-wins evaluation.
        let def = make_def(
            vec![],
            vec!["src/secrets/**"],
            &[("default", &["src/**"], &["src/**"])],
        );
        let engine = PolicyEngine::from_def(&def).expect("engine");

        let result = engine
            .check("default", FsOperation::Modify, "src/secrets/key.txt")
            .expect("check");

        assert!(
            !result.allowed,
            "global denyModify must override profile-level modify allow"
        );
    }

    #[test]
    fn check_global_deny_read_overrides_profile_read_allow() {
        let def = make_def(
            vec!["src/secrets/**"],
            vec![],
            &[("default", &["src/**"], &["src/**"])],
        );
        let engine = PolicyEngine::from_def(&def).expect("engine");

        let result = engine
            .check("default", FsOperation::Read, "src/secrets/key.txt")
            .expect("check");

        assert!(
            !result.allowed,
            "global denyRead must override profile-level read allow"
        );
    }

    #[test]
    fn check_rejects_parent_traversal_for_read_and_modify_paths() {
        let def = make_def(vec![], vec![], &[("default", &["**"], &["**"])]);
        let engine = PolicyEngine::from_def(&def).expect("engine");

        for (operation, path) in [
            (FsOperation::Read, "../secret.txt"),
            (FsOperation::Read, "src/../secret.txt"),
            (FsOperation::Read, "..\\secret.txt"),
            (FsOperation::Read, "src\\..\\secret.txt"),
            (FsOperation::Modify, "../secret.txt"),
            (FsOperation::Modify, "src/../secret.txt"),
            (FsOperation::Modify, "..\\secret.txt"),
            (FsOperation::Modify, "src\\..\\secret.txt"),
        ] {
            let err = engine
                .check("default", operation, path)
                .expect_err("parent traversal must be rejected");

            assert!(
                matches!(err, OrbitError::InvalidInput(_)),
                "expected InvalidInput for {operation:?} `{path}`, got {err:?}"
            );
        }
    }

    #[test]
    fn check_accepts_valid_relative_paths_after_normalization() {
        let def = make_def(
            vec![],
            vec![],
            &[("default", &["src/lib.rs"], &["src/lib.rs"])],
        );
        let engine = PolicyEngine::from_def(&def).expect("engine");

        for (operation, path) in [
            (FsOperation::Read, "src/lib.rs"),
            (FsOperation::Read, "./src/lib.rs"),
            (FsOperation::Modify, "src/lib.rs"),
            (FsOperation::Modify, "./src/lib.rs"),
        ] {
            let result = engine
                .check("default", operation, path)
                .expect("valid relative path should check");

            assert!(result.allowed, "{operation:?} `{path}` should be allowed");
            assert_eq!(result.matched_rule, "src/lib.rs");
        }
    }

    #[test]
    fn check_unknown_profile_returns_error_not_silent_allow() {
        // Invariant: requesting an undefined profile name must surface a
        // structured error rather than silently allowing or silently denying.
        // (The `unrestricted` profile is a documented special case;
        // arbitrary names must not be.)
        let def = make_def(vec![], vec![], &[("default", &["src/**"], &["src/**"])]);
        let engine = PolicyEngine::from_def(&def).expect("engine");

        let err = engine
            .check("missing", FsOperation::Read, "src/foo.rs")
            .expect_err("unknown profile must error");

        assert!(matches!(err, OrbitError::InvalidInput(_)));
    }

    #[test]
    fn check_records_matched_rule_for_audit_attribution() {
        // Invariant: a matched positive rule is reflected in the result's
        // `matched_rule` field so audit consumers can attribute the decision
        // to a specific rule rather than a bare allow/deny.
        let def = make_def(
            vec![],
            vec![],
            &[("default", &["src/lib.rs", "src/**"], &[])],
        );
        let engine = PolicyEngine::from_def(&def).expect("engine");

        let result = engine
            .check("default", FsOperation::Read, "src/lib.rs")
            .expect("check");
        assert!(result.allowed);
        assert!(
            result.matched_rule == "src/lib.rs" || result.matched_rule == "src/**",
            "matched_rule must surface a positive rule from the profile, got `{}`",
            result.matched_rule
        );
    }

    #[test]
    fn check_unknown_profile_resolves_unrestricted_when_named_unrestricted() {
        // Invariant: the special `unrestricted` profile resolves to the
        // documented permissive defaults even when the policy doesn't define
        // it. This is the single named exception to the unknown-profile
        // error path.
        let def = make_def(vec![], vec![], &[]);
        let engine = PolicyEngine::from_def(&def).expect("engine");

        let result = engine
            .check("unrestricted", FsOperation::Read, "anywhere.rs")
            .expect("unrestricted profile resolves");
        assert!(result.allowed);
    }
}
