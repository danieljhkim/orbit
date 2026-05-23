//! Unit tests for `install` — sibling layout under commands/tests/.

use super::super::install::{
    CompanionIntegrity, SemanticInstallParams, checksum_from_manifest, resolve_download_source,
    run, sha256_hex, verify_release_checksum_signature_with_key,
};

use crate::companion::unsafe_companion_overrides_enabled;
use crate::{CompanionPaths, locate_companion, platform_companion_filename};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use tempfile::{TempDir, tempdir};

#[test]
#[cfg(unix)]
fn stale_installed_companion_is_replaced_and_reported_as_changed() {
    let _guard = EnvGuard::new();
    let fixture = InstallFixture::new();
    fixture.write_installed_companion("0.3.1", "old");
    fixture.write_source_companion(env!("CARGO_PKG_VERSION"), "fresh");

    let result = run(SemanticInstallParams {
        model: None,
        force: false,
    })
    .expect("install should replace stale companion");

    assert!(result.companion_changed);
    assert!(!result.model_installed);
    assert!(
        std::fs::read_to_string(fixture.paths.companion_path())
            .expect("read replaced companion")
            .contains("replacement-marker: fresh")
    );
    let json = serde_json::to_value(&result).expect("serialize result");
    assert_eq!(json["companion_changed"], true);
    assert_eq!(json.get("companion_installed"), None);
}

#[test]
#[cfg(unix)]
fn force_replaces_current_companion() {
    let _guard = EnvGuard::new();
    let fixture = InstallFixture::new();
    fixture.write_installed_companion(env!("CARGO_PKG_VERSION"), "old-current");
    fixture.write_source_companion(env!("CARGO_PKG_VERSION"), "forced-fresh");

    let result = run(SemanticInstallParams {
        model: None,
        force: true,
    })
    .expect("forced install should replace current companion");

    assert!(result.companion_changed);
    assert!(
        std::fs::read_to_string(fixture.paths.companion_path())
            .expect("read replaced companion")
            .contains("replacement-marker: forced-fresh")
    );
}

#[test]
#[cfg(unix)]
fn current_companion_is_left_in_place_without_force() {
    let _guard = EnvGuard::new();
    let fixture = InstallFixture::new();
    fixture.write_installed_companion(env!("CARGO_PKG_VERSION"), "kept-current");
    fixture.write_source_companion(env!("CARGO_PKG_VERSION"), "unused-source");

    let result = run(SemanticInstallParams {
        model: None,
        force: false,
    })
    .expect("current install should be accepted");

    assert!(!result.companion_changed);
    assert!(
        std::fs::read_to_string(fixture.paths.companion_path())
            .expect("read kept companion")
            .contains("replacement-marker: kept-current")
    );
}

#[test]
#[cfg(unix)]
fn checksum_mismatch_rejects_replacement_before_install() {
    let _guard = EnvGuard::new();
    let fixture = InstallFixture::new();
    fixture.write_installed_companion("0.3.1", "old");
    fixture.write_source_companion(env!("CARGO_PKG_VERSION"), "tampered");
    set_env("ORBIT_SEARCH_COMPANION_SHA256", &"0".repeat(64));

    let error = run(SemanticInstallParams {
        model: None,
        force: false,
    })
    .expect_err("checksum mismatch should reject install");

    assert!(
        error
            .to_string()
            .contains("companion checksum verification failed"),
        "{error}"
    );
    assert!(
        std::fs::read_to_string(fixture.paths.companion_path())
            .expect("read retained companion")
            .contains("replacement-marker: old")
    );
}

#[test]
#[cfg(unix)]
fn unsafe_url_override_is_rejected_without_explicit_opt_in() {
    let _guard = EnvGuard::new();
    let fixture = InstallFixture::new();
    fixture.write_installed_companion("0.3.1", "old");
    remove_env("ORBIT_SEARCH_COMPANION");
    remove_env("ORBIT_SEARCH_COMPANION_ALLOW_UNSAFE");
    set_env(
        "ORBIT_SEARCH_COMPANION_URL",
        "http://example.invalid/companion",
    );

    let error = run(SemanticInstallParams {
        model: None,
        force: true,
    })
    .expect_err("http override should be rejected before download");

    assert!(error.to_string().contains("must use https"), "{error}");
    assert!(
        std::fs::read_to_string(fixture.paths.companion_path())
            .expect("read retained companion")
            .contains("replacement-marker: old")
    );
}

#[test]
fn default_download_source_requires_release_checksum_manifest() {
    let _guard = EnvGuard::new();
    remove_env("ORBIT_SEARCH_COMPANION");
    remove_env("ORBIT_SEARCH_COMPANION_URL");
    remove_env("ORBIT_SEARCH_COMPANION_SHA256");
    remove_env("ORBIT_SEARCH_COMPANION_ALLOW_UNSAFE");

    let source = resolve_download_source().expect("default source");

    match source.integrity {
        CompanionIntegrity::ReleaseSignedChecksum {
            checksums_url,
            signature_url,
            asset_name,
        } => {
            assert!(checksums_url.ends_with("/orbit-checksums.txt"));
            assert!(signature_url.ends_with("/orbit-checksums.txt.sig"));
            assert_eq!(asset_name, platform_companion_filename());
        }
        other => panic!("default source should require signed release checksum: {other:?}"),
    }
}

#[test]
fn checksum_manifest_selects_platform_asset() {
    let expected = "64ec88ca00b268e5ba1a35678a1b5316d212f4f366b2477232534a8aeca37f3c";
    let manifest = format!(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  other\n{expected}  ./{}\n",
        platform_companion_filename()
    );

    let checksum =
        checksum_from_manifest(&manifest, &platform_companion_filename()).expect("checksum entry");

    assert_eq!(checksum, expected);
}

#[test]
fn release_checksum_signature_verifies_trusted_manifest() {
    let manifest = "64ec88ca00b268e5ba1a35678a1b5316d212f4f366b2477232534a8aeca37f3c  orbit-search-companion-macos-aarch64";
    let signature = decode_hex(
        "7a30d97455a621a030176db6584a7a32875832e2d614b8faa63ac7ebd72f74100b7a9cdb3702c59d065c79fa653d0ca58dc90e19a0cd0a0699b28ff0699d89717a26d7605164d4653b6fa65d1b45db9c9a8532e47f85f3479ae6ca1175b90b3626410ebd6d511c480fdca8bce7e3eeef815d2276cfd5e8a3144a38856f5c4e21e4ee1e6962c9b3330a85e2f1a0c19c5ac921ad31394c65c2f4e477db79e901571b2a80721af082bfbb55d4c50e88ed07f69cf9434550c4625db23bc0df5b9aa4a56d683970e17b09db4a06f7544bc3b807a0c8ddda2a17e18a534fad22eca49da268f543dd510a7201a79f3259fbbe7c73d55212a93aa0a065bc07193ac81e02",
    );
    let public_key = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAsXuoUfxsHxU/MZ9ZiBos
oxyAiXDWb1QunevWWTT0++xvSxMJwTLkY2CIIQ7Bot17bMo/deoE5yqVDN9KJIzZ
/JeX2Mf/E4/4iUroCmZw6p/Z0Z2eHpZlg9mZMXviCUq6IDz1APzt6m1lV8lhZvD9
g3J50rxPizO91a30yUpYAIag02f9VPafyuuHA8SoDsWVwIyZ+Tazn7qAVKOgRWnU
Ua2MN0GwlGsg4AySpNCH1cDKvoRDMbB0ngEm/B9r6Yiqi4stJOdOSBL0Z2VEwfpZ
JXAFgkhiMRYeVULLjCacqxXMFDtH1J7uoowGuJaKUVA7fzq+vk2eBO8i1Wm0fVyK
iQIDAQAB
-----END PUBLIC KEY-----"#;

    verify_release_checksum_signature_with_key(manifest.as_bytes(), &signature, public_key)
        .expect("valid signature should verify");
    let tampered = manifest.replace("macos-aarch64", "linux-x86_64");
    let error =
        verify_release_checksum_signature_with_key(tampered.as_bytes(), &signature, public_key)
            .expect_err("tampered manifest should fail signature verification");

    assert!(
        error
            .to_string()
            .contains("release checksum signature verification failed"),
        "{error}"
    );
}

#[test]
fn unsafe_override_gate_accepts_mixed_case_values() {
    let _guard = EnvGuard::new();
    set_env("ORBIT_SEARCH_COMPANION_ALLOW_UNSAFE", "Yes");
    assert!(unsafe_companion_overrides_enabled());

    set_env("ORBIT_SEARCH_COMPANION_ALLOW_UNSAFE", "True");
    assert!(unsafe_companion_overrides_enabled());
}

#[test]
#[cfg(unix)]
fn locate_companion_prefers_managed_install_over_override() {
    let _guard = EnvGuard::new();
    let fixture = InstallFixture::new();
    fixture.write_installed_companion(env!("CARGO_PKG_VERSION"), "managed");
    fixture.write_source_companion(env!("CARGO_PKG_VERSION"), "override");

    let located = locate_companion().expect("managed companion should be located");

    assert_eq!(located, fixture.paths.companion_path());
}

#[test]
#[cfg(unix)]
fn runtime_override_requires_explicit_unsafe_opt_in() {
    let _guard = EnvGuard::new();
    let fixture = InstallFixture::new();
    fixture.write_source_companion(env!("CARGO_PKG_VERSION"), "override");
    remove_env("ORBIT_SEARCH_COMPANION_ALLOW_UNSAFE");

    let error = locate_companion().expect_err("override without unsafe gate should fail");

    assert!(
        error
            .to_string()
            .contains("ORBIT_SEARCH_COMPANION_ALLOW_UNSAFE"),
        "{error}"
    );
}

#[test]
#[cfg(unix)]
fn runtime_override_rejects_non_executable_path() {
    let _guard = EnvGuard::new();
    let fixture = InstallFixture::new();
    std::fs::write(&fixture.source_path, "#!/bin/sh\nexit 0\n").expect("write source");

    let error = locate_companion().expect_err("non-executable override should fail");

    assert!(error.to_string().contains("not executable"), "{error}");
}

#[test]
#[cfg(unix)]
fn path_candidates_are_not_executed_in_normal_lookup() {
    let _guard = EnvGuard::new();
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&home).expect("create home");
    std::fs::create_dir_all(&bin).expect("create bin");
    write_mock_companion(
        &bin.join("orbit-search-companion"),
        env!("CARGO_PKG_VERSION"),
        "path",
    );
    set_env("HOME", &home.to_string_lossy());
    set_env("USERPROFILE", &home.to_string_lossy());
    set_env("PATH", &bin.to_string_lossy());
    remove_env("ORBIT_SEARCH_COMPANION");
    remove_env("ORBIT_SEARCH_COMPANION_ALLOW_UNSAFE");

    let error = locate_companion().expect_err("PATH candidate should not be used");

    assert!(
        error.to_string().contains("Semantic search not enabled"),
        "{error}"
    );
}

struct InstallFixture {
    _temp: TempDir,
    paths: CompanionPaths,
    source_path: PathBuf,
}

impl InstallFixture {
    fn new() -> Self {
        let temp = tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let source_path = temp.path().join("source-companion");
        std::fs::create_dir_all(&home).expect("create home");
        set_env("HOME", &home.to_string_lossy());
        set_env("USERPROFILE", &home.to_string_lossy());
        set_env("ORBIT_SEARCH_COMPANION", &source_path.to_string_lossy());
        set_env("ORBIT_SEARCH_COMPANION_ALLOW_UNSAFE", "1");
        remove_env("ORBIT_SEARCH_COMPANION_URL");
        remove_env("ORBIT_SEARCH_COMPANION_SHA256");

        let paths = CompanionPaths::default_under_home().expect("paths");
        std::fs::create_dir_all(&paths.bin_dir).expect("create bin");
        let model_dir = paths.model_dir(crate::default_model().alias);
        std::fs::create_dir_all(&model_dir).expect("create model dir");
        std::fs::write(model_dir.join("orbit-model.json"), "{}").expect("write marker");

        Self {
            _temp: temp,
            paths,
            source_path,
        }
    }

    #[cfg(unix)]
    fn write_installed_companion(&self, version: &str, marker: &str) {
        write_mock_companion(&self.paths.companion_path(), version, marker);
        write_test_companion_integrity(&self.paths.companion_path(), version);
    }

    #[cfg(unix)]
    fn write_source_companion(&self, version: &str, marker: &str) {
        write_mock_companion(&self.source_path, version, marker);
    }
}

struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    vars: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn new() -> Self {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let lock = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let names = [
            "HOME",
            "USERPROFILE",
            "PATH",
            "ORBIT_SEARCH_COMPANION",
            "ORBIT_SEARCH_COMPANION_URL",
            "ORBIT_SEARCH_COMPANION_SHA256",
            "ORBIT_SEARCH_COMPANION_ALLOW_UNSAFE",
        ];
        let vars = names
            .into_iter()
            .map(|name| (name, std::env::var(name).ok()))
            .collect::<Vec<_>>();
        Self { _lock: lock, vars }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in &self.vars {
            match value {
                Some(value) => set_env(name, value),
                None => remove_env(name),
            }
        }
    }
}

#[cfg(unix)]
fn write_mock_companion(path: &std::path::Path, version: &str, marker: &str) {
    use std::os::unix::fs::PermissionsExt;

    let script = format!(
        r#"#!/bin/sh
# replacement-marker: {marker}
if [ "$1" = "--version-info" ]; then
  printf '%s\n' '{{"id":0,"result":{{"model_id":"bge-small-en-v1.5","dim":0,"max_input_tokens":0,"version":"{version}"}}}}'
  exit 0
fi
exit 0
"#
    );
    std::fs::write(path, script).expect("write companion");
    let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("chmod companion");
}

#[cfg(unix)]
fn write_test_companion_integrity(path: &std::path::Path, version: &str) {
    let bytes = std::fs::read(path).expect("read companion for checksum");
    let checksum = sha256_hex(&bytes);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("companion file name");
    let manifest_path = path.with_file_name(format!("{file_name}.sha256"));
    std::fs::write(
        manifest_path,
        serde_json::json!({
            "version": version,
            "sha256": checksum,
        })
        .to_string(),
    )
    .expect("write companion integrity");
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks(2)
        .map(|chunk| {
            let hex = std::str::from_utf8(chunk).expect("hex is utf8");
            u8::from_str_radix(hex, 16).expect("hex byte")
        })
        .collect()
}

fn set_env(name: &str, value: &str) {
    // SAFETY: tests that mutate process environment hold EnvGuard's global lock.
    unsafe { std::env::set_var(name, value) }
}

fn remove_env(name: &str) {
    // SAFETY: tests that mutate process environment hold EnvGuard's global lock.
    unsafe { std::env::remove_var(name) }
}
