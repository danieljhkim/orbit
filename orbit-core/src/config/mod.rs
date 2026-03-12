mod bootstrap;
mod persistence;
mod raw;
mod runtime;

pub(crate) use bootstrap::{default_config_template_for_root, seed_default_config};
pub(crate) use persistence::{PersistenceConfig, PersistenceType};
pub(crate) use runtime::{
    CodexExecutionPolicy, ExecutionEnvPolicy, RuntimeConfig, normalize_pass_list,
};

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::PersistenceConfig;
    use super::runtime::{CodexExecutionPolicy, ExecutionEnvPolicy, normalize_pass_list};

    #[test]
    fn normalize_pass_list_rejects_invalid_identifiers() {
        let err = normalize_pass_list(vec!["1INVALID".to_string()]).expect_err("must fail");
        assert!(err.to_string().contains("invalid variable name"));
    }

    #[test]
    fn normalize_pass_list_dedupes_and_sorts() {
        let values = normalize_pass_list(vec![
            "PATH".to_string(),
            "HOME".to_string(),
            "PATH".to_string(),
        ])
        .expect("normalize");
        assert_eq!(values, vec!["HOME".to_string(), "PATH".to_string()]);
    }

    #[test]
    fn default_pass_list_includes_cross_platform_system_vars() {
        let policy = ExecutionEnvPolicy::default();
        let pass = policy.pass();
        // Core cross-platform vars
        assert!(pass.contains(&"HOME".to_string()));
        assert!(pass.contains(&"PATH".to_string()));
        assert!(
            pass.contains(&"TMPDIR".to_string()),
            "TMPDIR must be in default pass list"
        );
        assert!(
            pass.contains(&"USER".to_string()),
            "USER must be in default pass list"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn default_pass_list_includes_macos_cf_encoding_var() {
        let policy = ExecutionEnvPolicy::default();
        let pass = policy.pass();
        assert!(
            pass.contains(&"__CF_USER_TEXT_ENCODING".to_string()),
            "__CF_USER_TEXT_ENCODING must be in default pass list on macOS"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn default_pass_list_excludes_macos_cf_encoding_var_on_non_macos() {
        let policy = ExecutionEnvPolicy::default();
        let pass = policy.pass();
        assert!(
            !pass.contains(&"__CF_USER_TEXT_ENCODING".to_string()),
            "__CF_USER_TEXT_ENCODING should not be in default pass list on non-macOS"
        );
    }

    #[test]
    fn codex_execution_defaults_to_workspace_write_without_approval_override() {
        let policy = CodexExecutionPolicy::default();
        assert_eq!(policy.sandbox(), "workspace-write");
        assert_eq!(policy.approval_policy(), None);
    }

    #[test]
    fn persistence_defaults_to_file_for_activities_and_uses_sqlite_for_audit() {
        let config = PersistenceConfig::default_for_data_root(Path::new("/tmp/orbit"));
        assert_eq!(config.job.path, std::path::PathBuf::from("/tmp/orbit/jobs"));
        assert_eq!(
            config.activity.path,
            std::path::PathBuf::from("/tmp/orbit/activities")
        );
        assert_eq!(config.job.format.as_deref(), Some("yaml"));
        assert_eq!(config.activity.format.as_deref(), Some("yaml"));
        assert_eq!(config.task, std::path::PathBuf::from("/tmp/orbit/tasks"));
        assert_eq!(config.skill, std::path::PathBuf::from("/tmp/orbit/skills"));
        assert_eq!(
            config.audit.path,
            std::path::PathBuf::from("/tmp/orbit/orbit.db")
        );
        assert_eq!(
            config.audit.persistence_type,
            super::PersistenceType::Sqlite
        );
    }
}
