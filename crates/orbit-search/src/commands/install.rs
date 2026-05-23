use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use orbit_common::types::OrbitError;
use reqwest::Url;
use rsa::RsaPublicKey;
use rsa::pkcs1v15::{Signature as RsaSignature, VerifyingKey};
use rsa::pkcs8::DecodePublicKey;
use rsa::signature::Verifier;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::commands::{DEFAULT_RELEASE_BASE_URL, parse_model};
use crate::companion::{
    COMPANION_OVERRIDE_ENV, UNSAFE_COMPANION_OVERRIDE_ENV, unsafe_companion_overrides_enabled,
    validate_companion_override_path, validate_managed_companion_path,
};
use crate::{CompanionPaths, platform_companion_filename};

const COMPANION_URL_ENV: &str = "ORBIT_SEARCH_COMPANION_URL";
const COMPANION_SHA256_ENV: &str = "ORBIT_SEARCH_COMPANION_SHA256";
const RELEASE_CHECKSUMS_FILENAME: &str = "orbit-checksums.txt";
const RELEASE_CHECKSUMS_SIGNATURE_FILENAME: &str = "orbit-checksums.txt.sig";
// Matches the release checksum signing key shipped by install.sh / npm installers.
const RELEASE_CHECKSUM_PUBLIC_KEY_PEM: &str = r#"-----BEGIN PUBLIC KEY-----
MIIBojANBgkqhkiG9w0BAQEFAAOCAY8AMIIBigKCAYEAuZ8vNa+DusYhrFBXNhBh
RSqn81AYe7tYEtCKImWGuy/6ziMHqDzDKHSku0sBMwcdLXBzI0RjNBacLCbbYr4H
icmYrsKqqfLGf+CWfrqDqY9d3hwUPtVMRp/ynVNW6nwKAmNl5dTgUc6ZBAZTtQtt
qwMD1JIOsrJ3vVDL9o3alcXcg/RyL0pGUo+vep2QZOjXnCGoJN3NeytQHag3zJyd
Wq4psc7j2H1Nb5EoyY/I/7vpdwME3Mrv2ffwtDmr0/+73q1yWUDf4btY9Ba7sOhE
Ir2UHm3bEboo1ErAYjjiDDuF/NjzZcZpJtuNbdj0vI7pHDyDZ7sKiEX7RkUO+e2c
IouiSfRJRrwnjpuergrq3ehNjkxcn5dFST1l23FOXGsy4F7ilrF6P9cgaAsE8dc7
CS9YgUE1ErfGJLZtfDDGKs6+E+7JiC1C3z7xwmfzOgv9gEvSlfrx2BbGl8esypKm
pYZDkW2dLqPeFj/WwGhZoYFHv0GOMIWdi6FNriQdkn4RAgMBAAE=
-----END PUBLIC KEY-----"#;

#[derive(Debug, Clone)]
pub struct SemanticInstallParams {
    pub model: Option<String>,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticInstallResult {
    pub companion_path: String,
    pub companion_changed: bool,
    pub model_id: String,
    pub model_installed: bool,
}

pub fn run(params: SemanticInstallParams) -> Result<SemanticInstallResult, OrbitError> {
    let spec = parse_model(params.model.as_deref())?;
    let paths = CompanionPaths::default_under_home()?;
    fs::create_dir_all(&paths.bin_dir).map_err(|error| OrbitError::Io(error.to_string()))?;
    fs::create_dir_all(&paths.models_dir).map_err(|error| OrbitError::Io(error.to_string()))?;
    emit_stale_companion_hint(&paths);

    let companion_path = paths.companion_path();
    let companion_changed = if params.force || companion_needs_install(&companion_path) {
        install_companion(&companion_path)?;
        true
    } else {
        false
    };

    let model_dir = paths.model_dir(spec.alias);
    let marker_path = model_dir.join("orbit-model.json");
    let model_installed = if marker_path.exists() {
        false
    } else {
        fs::create_dir_all(&model_dir).map_err(|error| OrbitError::Io(error.to_string()))?;
        download_model_with_companion(&companion_path, spec.alias, &model_dir)?;
        true
    };
    fs::write(&paths.active_model_path, spec.alias)
        .map_err(|error| OrbitError::Io(error.to_string()))?;

    Ok(SemanticInstallResult {
        companion_path: companion_path.to_string_lossy().to_string(),
        companion_changed,
        model_id: spec.alias.to_string(),
        model_installed,
    })
}

fn install_companion(destination: &Path) -> Result<(), OrbitError> {
    let temp_path = temporary_companion_path(destination)?;
    if temp_path.exists() {
        fs::remove_file(&temp_path).map_err(|error| OrbitError::Io(error.to_string()))?;
    }

    let install_result = install_companion_to_temp(&temp_path).and_then(|checksum| {
        replace_companion(&temp_path, destination)?;
        write_companion_integrity(destination, &checksum)
    });
    if install_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    install_result
}

fn install_companion_to_temp(temp_path: &Path) -> Result<String, OrbitError> {
    if let Some(local_path) = env_var_non_empty(COMPANION_OVERRIDE_ENV) {
        return install_local_companion(Path::new(&local_path), temp_path);
    }

    let source = resolve_download_source()?;
    let bytes = download_bytes(&source.url)?;
    let checksum = verify_download_integrity(&bytes, &source.integrity)?;
    fs::write(temp_path, bytes).map_err(|error| OrbitError::Io(error.to_string()))?;
    make_executable(temp_path)?;
    Ok(checksum)
}

fn install_local_companion(source_path: &Path, temp_path: &Path) -> Result<String, OrbitError> {
    validate_companion_override_path(source_path)?;
    let bytes = fs::read(source_path).map_err(|error| OrbitError::Io(error.to_string()))?;
    let checksum = sha256_hex(&bytes);
    if let Some(expected) = env_var_non_empty(COMPANION_SHA256_ENV) {
        verify_sha256_digest(&checksum, &expected)?;
    }
    fs::write(temp_path, bytes).map_err(|error| OrbitError::Io(error.to_string()))?;
    make_executable(temp_path)?;
    Ok(checksum)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompanionDownloadSource {
    pub(crate) url: String,
    pub(crate) integrity: CompanionIntegrity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompanionIntegrity {
    ReleaseSignedChecksum {
        checksums_url: String,
        signature_url: String,
        asset_name: String,
    },
    Sha256(String),
    UnsafeDeveloperOverride,
}

pub(crate) fn resolve_download_source() -> Result<CompanionDownloadSource, OrbitError> {
    if let Some(url) = env_var_non_empty(COMPANION_URL_ENV) {
        validate_download_url(&url)?;
        if let Some(expected) = env_var_non_empty(COMPANION_SHA256_ENV) {
            return Ok(CompanionDownloadSource {
                url,
                integrity: CompanionIntegrity::Sha256(normalize_sha256(&expected)?),
            });
        }
        if unsafe_companion_overrides_enabled() {
            tracing::warn!(
                env_var = UNSAFE_COMPANION_OVERRIDE_ENV,
                url = %url,
                "unsafe companion download bypasses checksum verification"
            );
            return Ok(CompanionDownloadSource {
                url,
                integrity: CompanionIntegrity::UnsafeDeveloperOverride,
            });
        }
        return Err(OrbitError::InvalidInput(format!(
            "{COMPANION_URL_ENV} requires {COMPANION_SHA256_ENV}=<sha256>; set {UNSAFE_COMPANION_OVERRIDE_ENV}=1 only for developer-only unsigned downloads"
        )));
    }

    let asset_name = platform_companion_filename();
    let url = format!("{DEFAULT_RELEASE_BASE_URL}/{asset_name}");
    validate_download_url(&url)?;
    Ok(CompanionDownloadSource {
        url,
        integrity: CompanionIntegrity::ReleaseSignedChecksum {
            checksums_url: release_metadata_url(RELEASE_CHECKSUMS_FILENAME),
            signature_url: release_metadata_url(RELEASE_CHECKSUMS_SIGNATURE_FILENAME),
            asset_name,
        },
    })
}

fn release_metadata_url(filename: &str) -> String {
    format!(
        "{}/{}",
        DEFAULT_RELEASE_BASE_URL.trim_end_matches('/'),
        filename
    )
}

fn validate_download_url(url: &str) -> Result<(), OrbitError> {
    let parsed = Url::parse(url)
        .map_err(|error| OrbitError::InvalidInput(format!("invalid companion URL: {error}")))?;
    if parsed.scheme() != "https" {
        if unsafe_companion_overrides_enabled() {
            tracing::warn!(
                env_var = UNSAFE_COMPANION_OVERRIDE_ENV,
                url,
                scheme = parsed.scheme(),
                "unsafe companion download bypasses HTTPS enforcement"
            );
        } else {
            return Err(OrbitError::InvalidInput(format!(
                "companion downloads must use https; set {UNSAFE_COMPANION_OVERRIDE_ENV}=1 only for developer-only testing"
            )));
        }
    }
    Ok(())
}

fn download_bytes(url: &str) -> Result<Vec<u8>, OrbitError> {
    Ok(reqwest::blocking::get(url)
        .map_err(|error| OrbitError::Execution(format!("failed to download companion: {error}")))?
        .error_for_status()
        .map_err(|error| OrbitError::Execution(format!("failed to download companion: {error}")))?
        .bytes()
        .map_err(|error| {
            OrbitError::Execution(format!("failed to read companion download: {error}"))
        })?
        .to_vec())
}

fn download_checksum_manifest(url: &str) -> Result<Vec<u8>, OrbitError> {
    Ok(reqwest::blocking::get(url)
        .map_err(|error| {
            OrbitError::Execution(format!(
                "failed to download companion checksum manifest: {error}"
            ))
        })?
        .error_for_status()
        .map_err(|error| {
            OrbitError::Execution(format!(
                "failed to download companion checksum manifest: {error}"
            ))
        })?
        .bytes()
        .map_err(|error| {
            OrbitError::Execution(format!(
                "failed to read companion checksum manifest: {error}"
            ))
        })?
        .to_vec())
}

fn download_checksum_signature(url: &str) -> Result<Vec<u8>, OrbitError> {
    Ok(reqwest::blocking::get(url)
        .map_err(|error| {
            OrbitError::Execution(format!(
                "failed to download companion checksum signature: {error}"
            ))
        })?
        .error_for_status()
        .map_err(|error| {
            OrbitError::Execution(format!(
                "failed to download companion checksum signature: {error}"
            ))
        })?
        .bytes()
        .map_err(|error| {
            OrbitError::Execution(format!(
                "failed to read companion checksum signature: {error}"
            ))
        })?
        .to_vec())
}

fn verify_download_integrity(
    bytes: &[u8],
    integrity: &CompanionIntegrity,
) -> Result<String, OrbitError> {
    let checksum = sha256_hex(bytes);
    match integrity {
        CompanionIntegrity::ReleaseSignedChecksum {
            checksums_url,
            signature_url,
            asset_name,
        } => {
            let manifest = download_checksum_manifest(checksums_url)?;
            let signature = download_checksum_signature(signature_url)?;
            verify_release_checksum_signature(&manifest, &signature)?;
            let manifest = std::str::from_utf8(&manifest).map_err(|error| {
                OrbitError::Execution(format!("companion checksum manifest is not UTF-8: {error}"))
            })?;
            let expected = checksum_from_manifest(manifest, asset_name)?;
            verify_sha256_digest(&checksum, &expected)?;
        }
        CompanionIntegrity::Sha256(expected) => verify_sha256_digest(&checksum, expected)?,
        CompanionIntegrity::UnsafeDeveloperOverride => {}
    }
    Ok(checksum)
}

fn verify_release_checksum_signature(manifest: &[u8], signature: &[u8]) -> Result<(), OrbitError> {
    verify_release_checksum_signature_with_key(manifest, signature, RELEASE_CHECKSUM_PUBLIC_KEY_PEM)
}

pub(crate) fn verify_release_checksum_signature_with_key(
    manifest: &[u8],
    signature: &[u8],
    public_key_pem: &str,
) -> Result<(), OrbitError> {
    let public_key = RsaPublicKey::from_public_key_pem(public_key_pem).map_err(|error| {
        OrbitError::Execution(format!(
            "failed to load trusted companion checksum signing key: {error}"
        ))
    })?;
    let signature = RsaSignature::try_from(signature).map_err(|error| {
        OrbitError::Execution(format!(
            "release checksum signature verification failed for {RELEASE_CHECKSUMS_FILENAME}: {error}"
        ))
    })?;
    let verifying_key = VerifyingKey::<Sha256>::new(public_key);
    verifying_key
        .verify(manifest, &signature)
        .map_err(|error| {
            OrbitError::Execution(format!(
                "release checksum signature verification failed for {RELEASE_CHECKSUMS_FILENAME}: {error}"
            ))
        })
}

pub(crate) fn checksum_from_manifest(
    manifest: &str,
    asset_name: &str,
) -> Result<String, OrbitError> {
    for line in manifest.lines() {
        let mut fields = line.split_whitespace();
        let Some(checksum) = fields.next() else {
            continue;
        };
        let Some(name) = fields.next() else {
            continue;
        };
        if checksum_manifest_name_matches(name, asset_name) {
            return normalize_sha256(checksum);
        }
    }
    Err(OrbitError::Execution(format!(
        "checksum entry for companion asset `{asset_name}` was not found in {RELEASE_CHECKSUMS_FILENAME}"
    )))
}

fn checksum_manifest_name_matches(name: &str, asset_name: &str) -> bool {
    name == asset_name
        || Path::new(name)
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .is_some_and(|file_name| file_name == asset_name)
}

fn verify_sha256_digest(actual: &str, expected: &str) -> Result<(), OrbitError> {
    let expected = normalize_sha256(expected)?;
    if actual != expected {
        return Err(OrbitError::Execution(format!(
            "companion checksum verification failed (expected {expected}, got {actual})"
        )));
    }
    Ok(())
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn normalize_sha256(value: &str) -> Result<String, OrbitError> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(OrbitError::InvalidInput(format!(
            "{COMPANION_SHA256_ENV} must be a 64-character hex SHA-256 digest"
        )));
    }
    Ok(normalized)
}

fn env_var_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn emit_stale_companion_hint(paths: &CompanionPaths) {
    let stale_path = paths.bin_dir.join(legacy_platform_companion_filename());
    if stale_path.exists() {
        let _ = writeln!(
            std::io::stderr().lock(),
            "stale companion detected at {}; remove it or run `orbit semantic install --force`",
            stale_path.display()
        );
    }
}

fn legacy_platform_companion_filename() -> String {
    let base = concat!("orbit-", "embed", "-companion");
    if cfg!(windows) {
        format!("{base}-{}.exe", crate::platform_id())
    } else {
        format!("{base}-{}", crate::platform_id())
    }
}

fn companion_needs_install(path: &Path) -> bool {
    if !path.exists() || validate_managed_companion_path(path).is_err() {
        return true;
    }
    // L-0036: Avoid native version probes; the sidecar lets us decide "install needed"
    // without executing an untrusted binary, but is not a tamper-detection mechanism.
    !installed_companion_integrity_matches(path).unwrap_or(false)
}

#[derive(Debug, Deserialize, Serialize)]
struct CompanionIntegrityManifest {
    version: String,
    sha256: String,
}

fn installed_companion_integrity_matches(path: &Path) -> Result<bool, OrbitError> {
    let manifest_path = companion_integrity_path(path).ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "companion destination has no file name: {}",
            path.display()
        ))
    })?;
    let manifest =
        fs::read_to_string(manifest_path).map_err(|error| OrbitError::Io(error.to_string()))?;
    let manifest: CompanionIntegrityManifest =
        serde_json::from_str(&manifest).map_err(|error| {
            OrbitError::InvalidInput(format!(
                "companion integrity manifest is not valid JSON: {error}"
            ))
        })?;
    let bytes = fs::read(path).map_err(|error| OrbitError::Io(error.to_string()))?;
    let checksum = sha256_hex(&bytes);
    Ok(manifest.version == env!("CARGO_PKG_VERSION")
        && normalize_sha256(&manifest.sha256)? == checksum)
}

pub(crate) fn write_companion_integrity(path: &Path, checksum: &str) -> Result<(), OrbitError> {
    let manifest_path = companion_integrity_path(path).ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "companion destination has no file name: {}",
            path.display()
        ))
    })?;
    let content = serde_json::to_string_pretty(&CompanionIntegrityManifest {
        version: env!("CARGO_PKG_VERSION").to_string(),
        sha256: checksum.to_string(),
    })
    .map(|json| format!("{json}\n"))
    .map_err(|error| {
        OrbitError::Execution(format!(
            "failed to serialize companion integrity manifest: {error}"
        ))
    })?;
    fs::write(manifest_path, content).map_err(|error| OrbitError::Io(error.to_string()))
}

fn companion_integrity_path(path: &Path) -> Option<std::path::PathBuf> {
    let file_name = path.file_name()?.to_string_lossy();
    Some(path.with_file_name(format!("{file_name}.sha256")))
}

fn temporary_companion_path(destination: &Path) -> Result<std::path::PathBuf, OrbitError> {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "companion destination has no file name: {}",
                destination.display()
            ))
        })?;
    Ok(destination.with_file_name(format!(".{file_name}.tmp-{}", std::process::id())))
}

#[cfg(unix)]
fn replace_companion(temp_path: &Path, destination: &Path) -> Result<(), OrbitError> {
    fs::rename(temp_path, destination).map_err(|error| OrbitError::Io(error.to_string()))
}

#[cfg(not(unix))]
fn replace_companion(temp_path: &Path, destination: &Path) -> Result<(), OrbitError> {
    if destination.exists() {
        fs::remove_file(destination).map_err(|error| OrbitError::Io(error.to_string()))?;
    }
    fs::rename(temp_path, destination).map_err(|error| OrbitError::Io(error.to_string()))
}

fn download_model_with_companion(
    companion_path: &Path,
    model: &str,
    model_dir: &Path,
) -> Result<(), OrbitError> {
    let status = Command::new(companion_path)
        .arg("--model")
        .arg(model)
        .arg("--model-path")
        .arg(model_dir)
        .arg("--download-model")
        .status()
        .map_err(|error| {
            OrbitError::Execution(format!(
                "failed to run search companion for model download: {error}"
            ))
        })?;
    if !status.success() {
        return Err(OrbitError::Execution(format!(
            "search companion failed to download model `{model}`"
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), OrbitError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|error| OrbitError::Io(error.to_string()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).map_err(|error| OrbitError::Io(error.to_string()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), OrbitError> {
    Ok(())
}
