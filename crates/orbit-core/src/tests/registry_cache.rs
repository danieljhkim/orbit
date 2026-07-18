use super::RegistryCacheService;

#[test]
fn compatibility_module_reexports_registry_cache_service() {
    let root = tempfile::tempdir().expect("tempdir");
    let service = RegistryCacheService::new(root.path());
    assert_eq!(
        service.cache_path(),
        root.path().join("registry-cache.json")
    );
}
