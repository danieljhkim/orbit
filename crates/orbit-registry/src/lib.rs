#![deny(clippy::print_stderr, clippy::print_stdout)]
//! Generic replicated registry substrate for Orbit publication flows.
//!
//! The crate intentionally works with opaque bytes and string keys. Consumer
//! crates choose their own payload schemas, then select a merge class that tells
//! registry transports how replicas may combine those payloads.

pub mod error;
pub mod merge;
pub mod transport;

pub use error::{RegistryError, RegistryResult};
pub use merge::MergeClass;
pub use transport::{NoopTransport, RegistryTransport, TransportEnvelope};

/// Identifies a source replica participating in registry publication.
pub trait Replica {
    /// Stable replica identifier used by transports and merge logs.
    fn replica_id(&self) -> &str;
}

/// Facade over a transport-backed replicated registry.
#[derive(Debug)]
pub struct Registry<T = NoopTransport> {
    transport: T,
}

impl Registry<NoopTransport> {
    /// Creates a registry with a no-op transport for callers that only need the
    /// type surface while wiring higher-level consumers.
    pub fn noop() -> Self {
        Self::new(NoopTransport)
    }
}

impl<T> Registry<T> {
    /// Creates a registry backed by the provided transport implementation.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Returns the backing transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Consumes the registry and returns the backing transport.
    pub fn into_transport(self) -> T {
        self.transport
    }
}

impl Default for Registry<NoopTransport> {
    fn default() -> Self {
        Self::noop()
    }
}

impl<T> Registry<T>
where
    T: RegistryTransport,
{
    /// Publishes opaque registry bytes for a replica under the selected merge
    /// class.
    pub fn publish<R>(
        &self,
        replica: &R,
        key: &str,
        merge_class: MergeClass,
        payload: &[u8],
    ) -> RegistryResult<()>
    where
        R: Replica,
    {
        self.transport.publish(TransportEnvelope {
            replica_id: replica.replica_id(),
            key,
            merge_class,
            payload,
        })
    }

    /// Fetches the latest transport-visible bytes for a key.
    pub fn fetch(&self, key: &str) -> RegistryResult<Option<Vec<u8>>> {
        self.transport.fetch(key)
    }
}
