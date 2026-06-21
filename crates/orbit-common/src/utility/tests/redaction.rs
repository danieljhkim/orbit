use std::ffi::{OsStr, OsString};

use super::super::redaction::{
    backfill_login_identity, is_high_confidence_single_token_credential, is_sensitive_env_name,
    os_login_name, redact_all,
};

fn values_for(vars: &[(OsString, OsString)], key: &str) -> Vec<OsString> {
    vars.iter()
        .filter(|(name, _)| name == OsStr::new(key))
        .map(|(_, value)| value.clone())
        .collect()
}

#[test]
fn redact_all_scrubs_key_query_params_case_insensitively() {
    let raw = concat!(
        "failed for url (https://example.test/v1beta/models/m:generateContent",
        "?key=AIzaSyQuerySecret&alt=sse) and ",
        "https://example.test/v1beta/cachedContents?foo=1&KEY=second-secret"
    );

    let redacted = redact_all(raw);

    assert!(!redacted.contains("AIzaSyQuerySecret"));
    assert!(!redacted.contains("second-secret"));
    assert!(redacted.contains("?key=[REDACTED_AUTH]&alt=sse"));
    assert!(redacted.contains("&KEY=[REDACTED_AUTH]"));
}

#[test]
fn redact_all_scrubs_provider_scm_cloud_tokens_and_connection_passwords() {
    let google = format!("AIza{}", "A".repeat(35));
    let gitlab = format!("glpat-{}", "B".repeat(20));
    let github_fine_grained = format!("github_pat_{}", "C".repeat(22));
    let github_oauth = format!("gho_{}", "D".repeat(36));
    let github_classic = format!("ghp_{}", "E".repeat(36));
    let github_server = format!("ghs_{}", "F".repeat(36));
    let github_user_server = format!("ghu_{}", "G".repeat(36));
    let github_refresh = format!("ghr_{}", "H".repeat(36));
    let aws_access_key_id = format!("AKIA{}", "1".repeat(16));
    let aws_secret_key = "aws_secret_access_key=awsSecretAccessKeyFixtureValue1234567890";
    let npm = format!("npm_{}", "I".repeat(36));
    let connection_string = "postgres://orbit_user:connection-pass@db.example.test/orbit";

    let raw = format!(
        "google={google}\n\
         gitlab={gitlab}\n\
         github_fine_grained={github_fine_grained}\n\
         github_oauth={github_oauth}\n\
         github_classic={github_classic}\n\
         github_server={github_server}\n\
         github_user_server={github_user_server}\n\
         github_refresh={github_refresh}\n\
         aws_access_key_id={aws_access_key_id}\n\
         {aws_secret_key}\n\
         npm={npm}\n\
         dsn={connection_string}"
    );

    let redacted = redact_all(&raw);

    for secret in [
        google.as_str(),
        gitlab.as_str(),
        github_fine_grained.as_str(),
        github_oauth.as_str(),
        github_classic.as_str(),
        github_server.as_str(),
        github_user_server.as_str(),
        github_refresh.as_str(),
        aws_access_key_id.as_str(),
        "awsSecretAccessKeyFixtureValue1234567890",
        npm.as_str(),
        "connection-pass",
    ] {
        assert!(!redacted.contains(secret), "{secret} was not redacted");
    }

    assert!(redacted.contains("postgres://orbit_user:[REDACTED_SECRET]@db.example.test/orbit"));
}

#[test]
fn high_confidence_single_token_detection_covers_provider_scm_cloud_families() {
    let credentials = [
        format!("AIza{}", "A".repeat(35)),
        format!("glpat-{}", "B".repeat(20)),
        format!("github_pat_{}", "C".repeat(22)),
        format!("gho_{}", "D".repeat(36)),
        format!("ghp_{}", "E".repeat(36)),
        format!("ghs_{}", "F".repeat(36)),
        format!("ghu_{}", "G".repeat(36)),
        format!("ghr_{}", "H".repeat(36)),
        format!("AKIA{}", "1".repeat(16)),
        "aws_secret_access_key=awsSecretAccessKeyFixtureValue1234567890".to_string(),
        format!("npm_{}", "I".repeat(36)),
        "postgres://orbit_user:connection-pass@db.example.test".to_string(),
    ];

    for credential in credentials {
        assert!(
            is_high_confidence_single_token_credential(&credential),
            "{credential} was not classified as a high-confidence credential"
        );
    }
}

#[test]
fn backfill_login_identity_fills_missing_user_and_logname() {
    let Some(expected) = os_login_name() else {
        // No resolvable login on this host; backfill is a no-op by design.
        return;
    };
    let mut vars = vec![(OsString::from("PATH"), OsString::from("/usr/bin"))];

    backfill_login_identity(&mut vars);

    assert_eq!(values_for(&vars, "USER"), vec![OsString::from(&expected)]);
    assert_eq!(
        values_for(&vars, "LOGNAME"),
        vec![OsString::from(&expected)]
    );
}

#[test]
fn backfill_login_identity_preserves_existing_nonempty_user() {
    let mut vars = vec![
        (OsString::from("USER"), OsString::from("explicit-user")),
        (OsString::from("LOGNAME"), OsString::from("explicit-user")),
    ];

    backfill_login_identity(&mut vars);

    assert_eq!(
        values_for(&vars, "USER"),
        vec![OsString::from("explicit-user")]
    );
    assert_eq!(
        values_for(&vars, "LOGNAME"),
        vec![OsString::from("explicit-user")]
    );
}

#[test]
fn backfill_login_identity_replaces_empty_user_without_duplicating() {
    let Some(expected) = os_login_name() else {
        return;
    };
    let mut vars = vec![(OsString::from("USER"), OsString::new())];

    backfill_login_identity(&mut vars);

    // Exactly one USER entry, carrying the resolved login (no empty leftover).
    assert_eq!(values_for(&vars, "USER"), vec![OsString::from(&expected)]);
}

#[test]
fn identity_backfill_does_not_weaken_credential_scrubbing() {
    // ORB-00409 AC#4: known credential-shaped names stay classified sensitive,
    // so they remain excluded from `non_sensitive_env_vars()` output.
    for name in [
        "ANTHROPIC_API_KEY",
        "GH_TOKEN",
        "MY_SECRET",
        "DB_PASSWORD",
        "AWS_SECRET_ACCESS_KEY",
        "SOME_PRIVATE_KEY",
        "AUTH_BEARER",
    ] {
        assert!(
            is_sensitive_env_name(name),
            "{name} must be classified sensitive (excluded from the forwarded env)"
        );
    }
    // Identity / runtime-context vars are NOT sensitive — they pass through and
    // are the ones the backfill guarantees.
    for name in ["USER", "LOGNAME", "HOME", "PATH"] {
        assert!(
            !is_sensitive_env_name(name),
            "{name} must not be classified sensitive"
        );
    }
}
