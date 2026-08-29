use super::super::ssh_auth::{
    FORCED_COMMAND_RESTRICTIONS, KeyObservation, ObservedKeys, SshAcceptance,
    auth_info_fingerprints, fingerprint_defect, observe_authenticating_keys, parse_public_key,
};

/// A real `ssh-keygen -t ed25519` key, kept beside the fingerprint
/// `ssh-keygen -l` printed for it. The pair is the whole point of the fixture:
/// an operator compares Orbit's output with their own tools, so a fingerprint
/// Orbit merely computes self-consistently would be worthless.
const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINMX3zk7E9dEvV0tMWx6b+FKAWBcQiweXKgUOc0AqkKH caller@daniels-mac-mini";
const KEY_FINGERPRINT: &str = "SHA256:5HTlLtSRdZg7lKPho8slfRr2Q1QTPuko05+KRX/8PQw";

#[test]
fn a_fingerprint_is_the_one_ssh_keygen_prints() {
    let key = parse_public_key(KEY).expect("a public key parses");

    assert_eq!(key.algorithm, "ssh-ed25519");
    assert_eq!(key.comment.as_deref(), Some("caller@daniels-mac-mini"));
    assert_eq!(
        key.fingerprint().expect("fingerprint"),
        KEY_FINGERPRINT,
        "Orbit's fingerprint must equal `ssh-keygen -l -f <key>.pub`, or an operator cannot \
         transcribe one into the other"
    );
}

#[test]
fn a_comment_and_trailing_whitespace_do_not_change_the_key() {
    let bare = parse_public_key(
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINMX3zk7E9dEvV0tMWx6b+FKAWBcQiweXKgUOc0AqkKH\n",
    )
    .expect("a comment-less key parses");

    assert_eq!(bare.comment, None);
    assert_eq!(bare.fingerprint().expect("fingerprint"), KEY_FINGERPRINT);
}

#[test]
fn the_rendered_line_forces_this_machines_own_argv() {
    let key = parse_public_key(KEY).expect("a public key parses");

    let line = key.authorized_keys_line("/usr/local/bin/orbit", "hm_alpha");

    assert!(
        line.starts_with(
            "command=\"/usr/local/bin/orbit mcp serve --accept-ssh --caller hm_alpha\","
        ),
        "the destination composes the whole argv, absolute path included: {line}"
    );
    assert!(line.contains(FORCED_COMMAND_RESTRICTIONS), "{line}");
    assert!(line.contains("no-pty"), "{line}");
    assert!(
        line.ends_with(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINMX3zk7E9dEvV0tMWx6b+FKAWBcQiweXKgUOc0AqkKH \
             caller@daniels-mac-mini"
        ),
        "{line}"
    );
    assert!(
        !line.contains("--operator"),
        "the line must not hand out operator authority; the callers file decides that: {line}"
    );
}

#[test]
fn a_private_key_or_an_options_line_is_refused() {
    for contents in [
        "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n",
        // An authorized_keys line, options first: composing a new grant out of
        // an old one must not silently drop the old one's restrictions.
        &format!("command=\"/bin/false\",no-pty {KEY}"),
        "",
        "# only a comment\n",
        "ssh-ed25519\n",
    ] {
        assert!(
            parse_public_key(contents).is_err(),
            "must refuse: {contents}"
        );
    }
}

#[test]
fn only_a_sha256_fingerprint_may_pin_a_row() {
    assert_eq!(fingerprint_defect(KEY_FINGERPRINT), None);
    assert_eq!(
        fingerprint_defect(&format!("{KEY_FINGERPRINT}=")),
        None,
        "padding an operator's paste may carry is not a defect"
    );

    // The plausible mistake: `ssh-keygen -E md5` prints this shape.
    assert!(fingerprint_defect("MD5:ab:cd:ef").is_some());
    assert!(fingerprint_defect("5HTlLtSRdZg7lKPho8slfRr2Q1QTPuko05+KRX/8PQw").is_some());
    assert!(fingerprint_defect("SHA256:not base64 at all!").is_some());
    assert!(
        fingerprint_defect("SHA256:c2hvcnQ").is_some(),
        "a digest that is not 32 bytes cannot be a SHA-256 one"
    );
}

#[test]
fn a_pin_matches_regardless_of_padding_or_prefix_transcription() {
    let observed = ObservedKeys {
        fingerprints: vec![KEY_FINGERPRINT.to_string()],
        observation: KeyObservation::AuthInfoFile,
    };

    assert!(observed.matches(KEY_FINGERPRINT));
    assert!(observed.matches(&format!("{KEY_FINGERPRINT}=")));
    assert!(observed.matches(&format!("  {KEY_FINGERPRINT}  ")));
    assert!(
        !observed.matches("SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        "a different key is a different key"
    );
}

#[test]
fn an_authorized_keys_command_may_supply_the_fingerprint_directly() {
    let observed =
        observe_authenticating_keys(Some(KEY_FINGERPRINT)).expect("a supplied fingerprint is one");

    assert_eq!(observed.observation, KeyObservation::ForcedCommandArgv);
    assert!(observed.matches(KEY_FINGERPRINT));
}

#[test]
fn sshds_auth_info_yields_the_keys_that_authenticated() {
    let fingerprints = auth_info_fingerprints(
        "publickey ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINMX3zk7E9dEvV0tMWx6b+FKAWBcQiweXKgUOc0AqkKH\n",
    );

    assert_eq!(fingerprints, vec![KEY_FINGERPRINT.to_string()]);
}

#[test]
fn a_session_that_authenticated_without_a_key_has_no_key_to_pin() {
    assert!(auth_info_fingerprints("password\n").is_empty());
    assert!(auth_info_fingerprints("").is_empty());
    assert!(
        auth_info_fingerprints("publickey ssh-ed25519 not-base64!\n").is_empty(),
        "an unparseable blob is not a key this destination observed"
    );
}

#[test]
fn an_ordinary_serve_cannot_name_a_caller() {
    // The rule "`--caller` is honored only under `--accept-ssh`" is carried by
    // the type: there is no Environment variant that can hold an identity, so
    // no downstream check can forget it.
    let acceptance = SshAcceptance::default();

    assert_eq!(acceptance, SshAcceptance::Environment);
}
