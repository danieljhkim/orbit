//! Atomic satellite registry cache [ORB-10267].
//!
//! A machine-local, versioned cache of one sanitized hub [`RegistrySnapshotV1`]
//! plus the local receipt time at which this machine accepted it. It is
//! **validation data only**: routine-pin and other validation reads may degrade
//! to warning-only when it is absent or stale, but ownership enforcement,
//! coordination-write routing, and knowledge-authoring enforcement must never
//! read it.
//!
//! Refresh holds one machine-global advisory lock across load, compare, and
//! write. The canonical hub payload is compared independently of the locally
//! stamped receipt: a different hub identity, a lower revision, or a different
//! canonical payload at equal revision is rejected without changing the prior
//! bytes; equal revision plus identical payload renews only the local receipt.
//! Writes use the crash-safe atomic-write primitive, so a failed refresh leaves
//! the previous valid snapshot readable, and after a commit error the cache is
//! reopened to distinguish pre-rename preservation from post-rename durability
//! uncertainty — only a complete old or complete new snapshot is ever
//! observable.

use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::{
    OrbitError, REGISTRY_CACHE_SCHEMA_VERSION, REGISTRY_SNAPSHOT_SCHEMA_VERSION, RegistryCacheV1,
    RegistrySnapshotV1, validate_machine_id,
};
use orbit_common::utility::fs::{atomic_write_bytes, with_exclusive_file_lock};

/// File under the global Orbit root holding the satellite registry cache.
pub const REGISTRY_CACHE_FILE: &str = "registry-cache.json";

/// Machine-local registry cache service bound to a single cache file.
#[derive(Debug, Clone)]
pub struct RegistryCacheService {
    cache_path: PathBuf,
}

/// The distinguishable states of a load, computed from the local receipt and an
/// explicit freshness threshold. Malformed and unsupported-future input is
/// never rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryCacheState {
    /// No cache file exists yet.
    Missing,
    /// A valid cache whose local receipt age is within the threshold.
    Current {
        cache: Box<RegistryCacheV1>,
        age_seconds: u64,
    },
    /// A valid cache whose local receipt age exceeds the threshold.
    Stale {
        cache: Box<RegistryCacheV1>,
        age_seconds: u64,
    },
    /// A present but unreadable cache (invalid JSON or the wrong shape). Never
    /// rewritten by a load.
    Malformed { reason: String },
    /// A cache written by a newer binary (higher schema version). Never
    /// rewritten by an older binary.
    UnsupportedFuture { schema_version: u32 },
}

/// The outcome of a successful refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryCacheOutcome {
    /// A new snapshot (first write or a higher revision) was committed.
    Written { revision: u64 },
    /// The incoming payload equalled the cached payload at the same revision;
    /// only the local receipt was renewed.
    ReceiptRenewed { revision: u64 },
}

impl RegistryCacheService {
    /// Bind the service to `<global_root>/registry-cache.json`.
    pub fn new(global_root: &Path) -> Self {
        Self {
            cache_path: global_root.join(REGISTRY_CACHE_FILE),
        }
    }

    /// Path of the backing cache file.
    pub fn cache_path(&self) -> &Path {
        &self.cache_path
    }

    /// Classify the cache without ever rewriting invalid input. Age is computed
    /// from the local receipt, never a remote clock.
    pub fn load(
        &self,
        now: DateTime<Utc>,
        freshness_threshold: Duration,
    ) -> Result<RegistryCacheState, OrbitError> {
        let Some(bytes) = read_optional(&self.cache_path)? else {
            return Ok(RegistryCacheState::Missing);
        };
        Ok(classify(&bytes, now, freshness_threshold))
    }

    /// Refresh the cache from a sanitized hub snapshot under one machine-global
    /// lock spanning load, compare, and write.
    pub fn refresh(
        &self,
        snapshot: RegistrySnapshotV1,
        now: DateTime<Utc>,
    ) -> Result<RegistryCacheOutcome, OrbitError> {
        self.refresh_with_writer(snapshot, now, atomic_write_bytes)
    }

    /// Refresh using the caller-provided commit primitive. Production refresh
    /// uses this seam with the atomic filesystem writer.
    pub(crate) fn refresh_with_writer<W>(
        &self,
        snapshot: RegistrySnapshotV1,
        now: DateTime<Utc>,
        writer: W,
    ) -> Result<RegistryCacheOutcome, OrbitError>
    where
        W: FnOnce(&Path, &[u8]) -> io::Result<()>,
    {
        self.refresh_with_codec(snapshot, now, serialize_cache, writer)
    }

    /// Refresh using explicit serialization and commit primitives. The
    /// writer-backed production path above delegates here.
    pub(crate) fn refresh_with_codec<S, W>(
        &self,
        snapshot: RegistrySnapshotV1,
        now: DateTime<Utc>,
        serializer: S,
        writer: W,
    ) -> Result<RegistryCacheOutcome, OrbitError>
    where
        S: FnOnce(&RegistryCacheV1) -> Result<Vec<u8>, OrbitError>,
        W: FnOnce(&Path, &[u8]) -> io::Result<()>,
    {
        with_exclusive_file_lock(&self.cache_path, "registry cache", || {
            self.refresh_locked(snapshot, now, serializer, writer)
        })
    }

    fn refresh_locked<S, W>(
        &self,
        snapshot: RegistrySnapshotV1,
        now: DateTime<Utc>,
        serializer: S,
        writer: W,
    ) -> Result<RegistryCacheOutcome, OrbitError>
    where
        S: FnOnce(&RegistryCacheV1) -> Result<Vec<u8>, OrbitError>,
        W: FnOnce(&Path, &[u8]) -> io::Result<()>,
    {
        validate_snapshot_schema(snapshot.schema_version)?;
        validate_snapshot_hub_identity(&snapshot, "incoming")?;
        let prior = self.read_valid_cache()?;
        if let Some(prior) = &prior {
            let outcome = compare_incoming(&prior.snapshot, &snapshot)?;
            if outcome == Comparison::RenewReceiptOnly {
                // The incoming snapshot may have different freshness/age
                // views derived at its later read time. Equal revision means
                // no canonical registry mutation occurred, so receipt renewal
                // must preserve the prior snapshot bytes and update only the
                // local receipt stamp.
                self.commit(&prior.snapshot, now, serializer, writer)?;
                return Ok(RegistryCacheOutcome::ReceiptRenewed {
                    revision: snapshot.registry_revision,
                });
            }
        }
        let revision = snapshot.registry_revision;
        self.commit(&snapshot, now, serializer, writer)?;
        Ok(RegistryCacheOutcome::Written { revision })
    }

    fn commit<S, W>(
        &self,
        snapshot: &RegistrySnapshotV1,
        now: DateTime<Utc>,
        serializer: S,
        writer: W,
    ) -> Result<(), OrbitError>
    where
        S: FnOnce(&RegistryCacheV1) -> Result<Vec<u8>, OrbitError>,
        W: FnOnce(&Path, &[u8]) -> io::Result<()>,
    {
        let cache = RegistryCacheV1 {
            schema_version: REGISTRY_CACHE_SCHEMA_VERSION,
            received_at: now,
            snapshot: snapshot.clone(),
        };
        let bytes = serializer(&cache)?;
        match writer(&self.cache_path, &bytes) {
            Ok(()) => Ok(()),
            Err(error) => Err(self.commit_failure_error(&cache, error)),
        }
    }

    /// After a write error, reopen the cache to distinguish honest outcomes:
    /// either the complete prior snapshot is preserved (the rename never
    /// happened) or the complete new snapshot is present but its durability is
    /// uncertain (the rename landed but fsync may not have). Never reports a
    /// torn read.
    fn commit_failure_error(&self, intended: &RegistryCacheV1, error: io::Error) -> OrbitError {
        match self.read_valid_cache() {
            Ok(Some(observed)) if observed == *intended => OrbitError::Store(format!(
                "registry cache write reported an error ({error}), but the complete new cache \
                     at revision {} is now readable; its durability is uncertain",
                intended.snapshot.registry_revision
            )),
            Ok(Some(observed)) => OrbitError::Store(format!(
                "registry cache write failed ({error}); the prior snapshot at revision {} is \
                 preserved",
                observed.snapshot.registry_revision
            )),
            Ok(None) => OrbitError::Store(format!(
                "registry cache write failed ({error}); no cache is present"
            )),
            Err(reopen) => OrbitError::Store(format!(
                "registry cache write failed ({error}); reopening to classify preservation also \
                 failed: {reopen}"
            )),
        }
    }

    fn read_valid_cache(&self) -> Result<Option<RegistryCacheV1>, OrbitError> {
        let Some(bytes) = read_optional(&self.cache_path)? else {
            return Ok(None);
        };
        match classify(&bytes, Utc::now(), Duration::zero()) {
            RegistryCacheState::Current { cache, .. } | RegistryCacheState::Stale { cache, .. } => {
                Ok(Some(*cache))
            }
            RegistryCacheState::Malformed { reason } => Err(OrbitError::Store(format!(
                "registry cache '{}' is malformed: {reason}",
                self.cache_path.display()
            ))),
            RegistryCacheState::UnsupportedFuture { schema_version } => {
                Err(OrbitError::InvalidInput(format!(
                    "registry cache '{}' has unsupported future schema_version {schema_version}; \
                     upgrade Orbit. The file is left unchanged",
                    self.cache_path.display()
                )))
            }
            RegistryCacheState::Missing => Ok(None),
        }
    }
}

fn serialize_cache(cache: &RegistryCacheV1) -> Result<Vec<u8>, OrbitError> {
    serde_json::to_vec_pretty(cache)
        .map_err(|error| OrbitError::Store(format!("serialize registry cache: {error}")))
}

fn validate_snapshot_schema(schema_version: u32) -> Result<(), OrbitError> {
    if schema_version > REGISTRY_SNAPSHOT_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "registry cache refresh rejected: incoming snapshot has unsupported future \
             schema_version {schema_version}; upgrade Orbit"
        )));
    }
    if schema_version != REGISTRY_SNAPSHOT_SCHEMA_VERSION {
        return Err(OrbitError::InvalidInput(format!(
            "registry cache refresh rejected: incoming snapshot schema_version {schema_version} \
             is unsupported; expected {REGISTRY_SNAPSHOT_SCHEMA_VERSION}"
        )));
    }
    Ok(())
}

fn validate_snapshot_hub_identity<'a>(
    snapshot: &'a RegistrySnapshotV1,
    source: &str,
) -> Result<&'a str, OrbitError> {
    let hub_machine_id = snapshot.hub_machine_id.as_deref().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "registry cache {source} snapshot omits required hub_machine_id"
        ))
    })?;
    validate_machine_id(hub_machine_id).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "registry cache {source} snapshot has invalid hub_machine_id '{hub_machine_id}': \
             {error}"
        ))
    })?;
    Ok(hub_machine_id)
}

#[derive(Debug, PartialEq, Eq)]
enum Comparison {
    AcceptNewer,
    RenewReceiptOnly,
}

/// Compare an incoming snapshot against the prior cached snapshot. Rejects a
/// different hub, a lower revision, or a different canonical payload at equal
/// revision before any write.
fn compare_incoming(
    prior: &RegistrySnapshotV1,
    incoming: &RegistrySnapshotV1,
) -> Result<Comparison, OrbitError> {
    let prior_hub = validate_snapshot_hub_identity(prior, "persisted")?;
    let incoming_hub = validate_snapshot_hub_identity(incoming, "incoming")?;
    if incoming_hub != prior_hub {
        return Err(OrbitError::InvalidInput(format!(
            "registry cache refresh rejected: incoming hub '{incoming_hub}' differs from cached \
             hub '{prior_hub}'"
        )));
    }
    if incoming.registry_revision < prior.registry_revision {
        return Err(OrbitError::InvalidInput(format!(
            "registry cache refresh rejected: incoming revision {} is lower than cached revision {}",
            incoming.registry_revision, prior.registry_revision
        )));
    }
    if incoming.registry_revision == prior.registry_revision {
        if incoming.canonical_payload_eq(prior) {
            return Ok(Comparison::RenewReceiptOnly);
        }
        return Err(OrbitError::InvalidInput(format!(
            "registry cache refresh rejected: incoming payload differs from cached payload at the \
             same revision {}",
            prior.registry_revision
        )));
    }
    Ok(Comparison::AcceptNewer)
}

/// Classify raw cache bytes. Distinguishes malformed and unsupported-future
/// input from a valid cache and never rewrites invalid input.
fn classify(bytes: &[u8], now: DateTime<Utc>, freshness_threshold: Duration) -> RegistryCacheState {
    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(error) => {
            return RegistryCacheState::Malformed {
                reason: format!("invalid JSON: {error}"),
            };
        }
    };
    let Some(schema_version) = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
    else {
        return RegistryCacheState::Malformed {
            reason: "missing or non-integer schema_version".to_string(),
        };
    };
    if schema_version > u64::from(REGISTRY_CACHE_SCHEMA_VERSION) {
        return RegistryCacheState::UnsupportedFuture {
            schema_version: schema_version as u32,
        };
    }
    if schema_version != u64::from(REGISTRY_CACHE_SCHEMA_VERSION) {
        return RegistryCacheState::Malformed {
            reason: format!(
                "unsupported registry cache schema_version {schema_version}; expected {}",
                REGISTRY_CACHE_SCHEMA_VERSION
            ),
        };
    }
    let Some(snapshot_schema_version) = value
        .get("snapshot")
        .and_then(|snapshot| snapshot.get("schema_version"))
        .and_then(serde_json::Value::as_u64)
    else {
        return RegistryCacheState::Malformed {
            reason: "missing or non-integer snapshot.schema_version".to_string(),
        };
    };
    if snapshot_schema_version > u64::from(REGISTRY_SNAPSHOT_SCHEMA_VERSION) {
        return RegistryCacheState::UnsupportedFuture {
            schema_version: snapshot_schema_version as u32,
        };
    }
    if snapshot_schema_version != u64::from(REGISTRY_SNAPSHOT_SCHEMA_VERSION) {
        return RegistryCacheState::Malformed {
            reason: format!(
                "unsupported registry snapshot schema_version {snapshot_schema_version}; \
                 expected {}",
                REGISTRY_SNAPSHOT_SCHEMA_VERSION
            ),
        };
    }
    let cache: RegistryCacheV1 = match serde_json::from_value(value) {
        Ok(cache) => cache,
        Err(error) => {
            return RegistryCacheState::Malformed {
                reason: format!("does not match the registry cache schema: {error}"),
            };
        }
    };
    if let Err(error) = validate_snapshot_hub_identity(&cache.snapshot, "persisted") {
        return RegistryCacheState::Malformed {
            reason: error.to_string(),
        };
    }
    let age_seconds = u64::try_from(
        now.signed_duration_since(cache.received_at)
            .num_seconds()
            .max(0),
    )
    .unwrap_or_default();
    if now.signed_duration_since(cache.received_at) > freshness_threshold {
        RegistryCacheState::Stale {
            cache: Box::new(cache),
            age_seconds,
        }
    } else {
        RegistryCacheState::Current {
            cache: Box::new(cache),
            age_seconds,
        }
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, OrbitError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(OrbitError::Io(format!(
            "read registry cache '{}': {error}",
            path.display()
        ))),
    }
}
