mod persistence;
mod raw;
mod runtime;

pub(crate) use persistence::{PersistenceConfig, PersistenceType};
pub(crate) use runtime::{ExecutionEnvPolicy, RuntimeConfig};

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::PersistenceConfig;
    use super::runtime::normalize_pass_list;

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
    fn persistence_defaults_to_file_for_jobs_and_works() {
        let config = PersistenceConfig::default_for_data_root(Path::new("/tmp/orbit"));
        assert_eq!(
            config.scheduler.path,
            std::path::PathBuf::from("/tmp/orbit/schedulers")
        );
        assert_eq!(config.job.path, std::path::PathBuf::from("/tmp/orbit/jobs"));
        assert_eq!(config.scheduler.format.as_deref(), Some("yaml"));
        assert_eq!(config.job.format.as_deref(), Some("yaml"));
    }
}
