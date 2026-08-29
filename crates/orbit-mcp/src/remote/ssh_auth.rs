//! What sshd told this destination about the session it started [ORB-11053].
//!
//! Tier 1 of destination-side caller authorization keys on a label the caller
//! chose, so it can only be an accident guard. This module supplies the
//! missing half: the caller identity is written by the *destination*, next to
//! a public key, in its own `authorized_keys`, and sshd will not run that
//! forced command for anyone who cannot complete the key exchange. Orbit holds
//! no credential of its own — the key is the one SSH already checks.
//!
//! Three things live here, and nothing else:
//!
//! 1. The `SHA256:` fingerprint of an SSH public key, in the form `ssh-keygen`
//!    and `sshd` print, so an operator can compare what Orbit says with what
//!    their own tools say without a conversion step.
//! 2. Observation of the key that actually authenticated this session, from
//!    the two places a destination can learn it.
//! 3. The `authorized_keys` line an operator installs. Orbit renders it and
//!    never writes it: that file governs shell access to the whole machine,
//!    which is far more than Orbit's business.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD as BASE64, STANDARD_NO_PAD as BASE64_NO_PAD};
use orbit_common::OrbitError;
use sha2::{Digest, Sha256};

/// Set by sshd to a file listing the authentication methods that succeeded,
/// when the destination's `sshd_config` has `ExposeAuthInfo yes`.
const SSH_USER_AUTH_ENV: &str = "SSH_USER_AUTH";

/// The command the *caller* asked for, which a forced command replaces.
///
/// Named here only so the one place that mentions it can say, in one spot,
/// that it is never read as input. See [`ignored_original_command`].
const SSH_ORIGINAL_COMMAND_ENV: &str = "SSH_ORIGINAL_COMMAND";

/// The prefix sshd and `ssh-keygen -l` use for a SHA-256 key fingerprint.
pub const FINGERPRINT_PREFIX: &str = "SHA256:";

/// The restrictions that ship with a rendered `authorized_keys` line.
///
/// A forced command alone still leaves the key able to forward ports and
/// allocate a PTY. The MCP transport needs none of that — it is one non-PTY
/// stdio pipe — so the line closes what it does not use. `no-pty` is also what
/// keeps the Tier 1 non-terminal check honest for anyone who reasons about
/// this key without reading the forced command.
pub const FORCED_COMMAND_RESTRICTIONS: &str =
    "no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding";

/// Whether the caller asked for a command that the forced command replaced.
///
/// The value is deliberately not returned, parsed, merged, or used to derive a
/// requested authority: under a forced command the destination composes its
/// own argv, and honoring any part of the caller's string would hand the
/// authority decision straight back to the caller. Only its *presence* is
/// reported, so the audit trail can show that something was overridden.
pub fn ignored_original_command() -> bool {
    std::env::var_os(SSH_ORIGINAL_COMMAND_ENV).is_some()
}

/// How this server process was started, as far as SSH is concerned.
///
/// The two variants are the two tiers. The distinction is a type rather than a
/// pair of flags because the rule it encodes — an identity is honored only
/// when the *destination* composed the argv that carries it — is exactly the
/// rule a `--caller` on an ordinary `orbit mcp serve` would break. Making that
/// combination unrepresentable is cheaper than checking for it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SshAcceptance {
    /// An ordinary `orbit mcp serve`. Whether the session is remote-originated
    /// is the destination's own environment observation, and the caller
    /// identity is at best a label the caller forwarded.
    #[default]
    Environment,
    /// A forced command in the destination's own `authorized_keys` ran this
    /// server, so sshd authenticated the key that selected it.
    ForcedCommand {
        /// The caller identity the destination wrote next to that key.
        caller: Option<String>,
        /// The authenticating key's fingerprint, when an
        /// `AuthorizedKeysCommand` expanded sshd's `%f` into the line.
        caller_key_fingerprint: Option<String>,
    },
}

/// An SSH public key as `authorized_keys` and `*.pub` files spell it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshPublicKey {
    /// Key algorithm, such as `ssh-ed25519`.
    pub algorithm: String,
    /// Base64 key blob.
    pub blob: String,
    /// Free-form trailing comment, conventionally `user@host`.
    pub comment: Option<String>,
}

impl SshPublicKey {
    /// The key's fingerprint, in the `SHA256:` form sshd and `ssh-keygen -l`
    /// print.
    pub fn fingerprint(&self) -> Result<String, OrbitError> {
        let blob = BASE64.decode(self.blob.as_bytes()).map_err(|error| {
            OrbitError::InvalidInput(format!("SSH public key is not valid base64: {error}"))
        })?;
        Ok(fingerprint_of_blob(&blob))
    }

    /// The `authorized_keys` line that pins `machine_id` to this key.
    ///
    /// `orbit_command` is the absolute path the destination will run, because
    /// sshd executes a forced command without a login shell's `PATH`. The
    /// destination requests operator so the matched callers-file grant can be
    /// realized; that grant remains the ceiling and may still cap or deny it.
    pub fn authorized_keys_line(&self, orbit_command: &str, machine_id: &str) -> String {
        let comment = self
            .comment
            .as_deref()
            .map(|comment| format!(" {comment}"))
            .unwrap_or_default();
        format!(
            "command=\"{orbit_command} mcp serve --accept-ssh --caller {machine_id} --operator\",\
             {FORCED_COMMAND_RESTRICTIONS} {algorithm} {blob}{comment}",
            algorithm = self.algorithm,
            blob = self.blob,
        )
    }
}

/// Parse one SSH public key, as written in a `*.pub` file.
///
/// Leading `authorized_keys` options are rejected rather than skipped: an
/// operator who passes an already-restricted line is composing a new grant
/// from an old one, and silently dropping the old options would produce a line
/// that grants more than the one it came from.
pub fn parse_public_key(contents: &str) -> Result<SshPublicKey, OrbitError> {
    let line = contents
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or_else(|| OrbitError::InvalidInput("SSH public key file is empty".to_string()))?;
    let mut fields = line.splitn(3, char::is_whitespace);
    let algorithm = fields.next().unwrap_or_default().to_string();
    let blob = fields.next().unwrap_or_default().trim().to_string();
    let comment = fields
        .next()
        .map(str::trim)
        .filter(|comment| !comment.is_empty())
        .map(ToOwned::to_owned);
    if !algorithm.starts_with("ssh-")
        && !algorithm.starts_with("ecdsa-")
        && !algorithm.starts_with("sk-")
    {
        return Err(OrbitError::InvalidInput(format!(
            "'{algorithm}' is not an SSH public key algorithm; pass the public half of the \
             caller's key, such as `~/.ssh/id_ed25519.pub`, not a private key or an \
             authorized_keys line with options"
        )));
    }
    if blob.is_empty() {
        return Err(OrbitError::InvalidInput(
            "SSH public key has no key material".to_string(),
        ));
    }
    let key = SshPublicKey {
        algorithm,
        blob,
        comment,
    };
    // Fail here rather than at the first comparison: a key that cannot be
    // fingerprinted cannot be pinned, and the operator is standing right here.
    key.fingerprint()?;
    Ok(key)
}

/// Why `candidate` is not a well-formed `SHA256:` fingerprint, if it is not.
///
/// Checked when the callers file loads so a fingerprint in the wrong format —
/// an `MD5:` one from `ssh-keygen -E md5`, most plausibly — is reported as the
/// malformed file it is, instead of silently never matching and presenting as
/// a key mismatch on every session. The detail is returned as a fragment
/// rather than an error so the loader can name the row it came from.
pub fn fingerprint_defect(candidate: &str) -> Option<String> {
    let candidate = candidate.trim();
    let Some(digest) = candidate.strip_prefix(FINGERPRINT_PREFIX) else {
        return Some(format!(
            "'{candidate}' is not a SHA-256 key fingerprint; use the `{FINGERPRINT_PREFIX}…` form \
             printed by `ssh-keygen -l -f <key>.pub`"
        ));
    };
    match BASE64_NO_PAD.decode(digest.trim_end_matches('=').as_bytes()) {
        Err(error) => Some(format!("'{candidate}' is not base64: {error}")),
        Ok(decoded) if decoded.len() != 32 => Some(format!(
            "'{candidate}' decodes to {} bytes, not the 32 a SHA-256 digest has",
            decoded.len()
        )),
        Ok(_) => None,
    }
}

/// Where a destination learned the key that authenticated this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyObservation {
    /// Supplied in the destination-composed argv, by an
    /// `AuthorizedKeysCommand` that expanded sshd's `%f` token.
    ForcedCommandArgv,
    /// Read from the file sshd wrote for `ExposeAuthInfo`.
    AuthInfoFile,
}

impl KeyObservation {
    /// How the source reads in an operator-facing message.
    pub fn label(self) -> &'static str {
        match self {
            Self::ForcedCommandArgv => "the forced command's --caller-key-fingerprint",
            Self::AuthInfoFile => "sshd's SSH_USER_AUTH",
        }
    }
}

/// The keys sshd accepted for this session, as this process can see them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedKeys {
    /// Every public key that authenticated, in `SHA256:` form. More than one
    /// is ordinary: a destination requiring two factors records each.
    pub fingerprints: Vec<String>,
    pub observation: KeyObservation,
}

impl ObservedKeys {
    /// Whether `pinned` is among the keys that authenticated.
    ///
    /// Padding and case in the base64 tail are normalized away so a
    /// fingerprint copied from `ssh-keygen`, from `sshd`'s log, or from this
    /// command's own output all compare equal.
    pub fn matches(&self, pinned: &str) -> bool {
        let pinned = normalize_fingerprint(pinned);
        self.fingerprints
            .iter()
            .any(|observed| normalize_fingerprint(observed) == pinned)
    }

    /// The observed keys, rendered for a refusal an operator has to act on.
    pub fn label(&self) -> String {
        if self.fingerprints.is_empty() {
            return "none".to_string();
        }
        self.fingerprints.join(", ")
    }
}

/// The key that authenticated this session, or `None` when the destination
/// cannot see it.
///
/// `None` is the ordinary case on a stock `sshd_config`: `ExposeAuthInfo` is
/// off by default. It means verification is *unavailable*, which is not the
/// same as a mismatch and must not be treated as one — a destination that
/// cannot observe the key still serves the session, and the audit trail
/// records the identity as key-bound or self-asserted on the strength of the
/// forced command alone.
pub fn observe_authenticating_keys(supplied_fingerprint: Option<&str>) -> Option<ObservedKeys> {
    if let Some(fingerprint) = supplied_fingerprint
        .map(str::trim)
        .filter(|fingerprint| !fingerprint.is_empty())
    {
        return Some(ObservedKeys {
            fingerprints: vec![fingerprint.to_string()],
            observation: KeyObservation::ForcedCommandArgv,
        });
    }
    let path = PathBuf::from(std::env::var_os(SSH_USER_AUTH_ENV)?);
    read_auth_info(&path)
}

/// The public keys sshd's `ExposeAuthInfo` file names, as fingerprints.
///
/// Every line is `<method> <detail…>`; only `publickey` lines carry a key, and
/// only their `<algorithm> <blob>` tail is of interest. More than one is
/// ordinary — a destination requiring two factors records each — and none is
/// the honest answer for a session that authenticated without a key.
pub fn auth_info_fingerprints(contents: &str) -> Vec<String> {
    contents
        .lines()
        .filter_map(|line| line.trim().strip_prefix("publickey "))
        .filter_map(|key| {
            let blob = key.split_whitespace().nth(1)?;
            BASE64.decode(blob.as_bytes()).ok()
        })
        .map(|blob| fingerprint_of_blob(&blob))
        .collect()
}

/// Read sshd's `ExposeAuthInfo` file, or report that it cannot be read.
///
/// An unreadable file reads as "not observable" rather than as an error: sshd
/// owns that file's lifetime, and refusing a session because a temporary file
/// was already cleaned up would refuse sessions no policy meant to refuse.
fn read_auth_info(path: &Path) -> Option<ObservedKeys> {
    let contents = std::fs::read_to_string(path)
        .inspect_err(|error| {
            tracing::debug!(
                target: "orbit.mcp.callers",
                path = %path.display(),
                %error,
                "SSH_USER_AUTH is set but unreadable; the authenticating key cannot be observed"
            );
        })
        .ok()?;
    let fingerprints = auth_info_fingerprints(&contents);
    (!fingerprints.is_empty()).then_some(ObservedKeys {
        fingerprints,
        observation: KeyObservation::AuthInfoFile,
    })
}

fn fingerprint_of_blob(blob: &[u8]) -> String {
    format!(
        "{FINGERPRINT_PREFIX}{}",
        BASE64_NO_PAD.encode(Sha256::digest(blob))
    )
}

fn normalize_fingerprint(fingerprint: &str) -> String {
    fingerprint
        .trim()
        .strip_prefix(FINGERPRINT_PREFIX)
        .unwrap_or(fingerprint.trim())
        .trim_end_matches('=')
        .to_string()
}
