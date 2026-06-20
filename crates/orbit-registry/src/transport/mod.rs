use crate::{MergeClass, RegistryResult};

/// Opaque publication unit passed to registry transports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransportEnvelope<'a> {
    /// Replica that produced the payload.
    pub replica_id: &'a str,
    /// Consumer-defined registry key.
    pub key: &'a str,
    /// Merge behavior requested for the payload.
    pub merge_class: MergeClass,
    /// Consumer-defined payload bytes.
    pub payload: &'a [u8],
}

/// Transport boundary for publishing and reading registry bytes.
pub trait RegistryTransport {
    /// Publishes an opaque envelope.
    fn publish(&self, envelope: TransportEnvelope<'_>) -> RegistryResult<()>;

    /// Fetches transport-visible bytes for a key.
    fn fetch(&self, key: &str) -> RegistryResult<Option<Vec<u8>>>;
}

/// Transport stub used until concrete transports land.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopTransport;

impl RegistryTransport for NoopTransport {
    fn publish(&self, _envelope: TransportEnvelope<'_>) -> RegistryResult<()> {
        Ok(())
    }

    fn fetch(&self, _key: &str) -> RegistryResult<Option<Vec<u8>>> {
        Ok(None)
    }
}
