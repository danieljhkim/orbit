1. Reproduce: write a job via 'orbit job add', then cat the YAML file to verify all fields are at 2-space indent under 'job:'.
2. Trace the serde_yaml serialization path: JobFileStore::write_activity -> serde_yaml::to_string(&JobFileDocument { schema_version, job }). Confirm JobFileDocument uses rename_all = camelCase and Job has no rename_all.
3. Identify why state/created_at/updated_at/env_extra ended up at top level. Likely a previous version of the Job struct was split into multiple structs and merged incorrectly.
4. Verify current code produces correct output (should be fixed by the struct cleanup already done).
5. Add a store integration test that reads back a written job and asserts all fields are present and correct (including state).