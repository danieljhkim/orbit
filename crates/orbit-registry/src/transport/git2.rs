use std::path::Path;

use crate::{
    RegistryResult,
    error::unsupported,
    transport::{RegistryTransport, TransportEnvelope},
};

/// Placeholder git2-backed registry transport.
#[derive(Debug)]
pub struct Git2Transport {
    repository_path: std::path::PathBuf,
}

impl Git2Transport {
    /// Opens a git repository for future registry publication work.
    pub fn open(repository_path: impl AsRef<Path>) -> RegistryResult<Self> {
        let repository_path = repository_path.as_ref();
        ::git2::Repository::open(repository_path)
            .map_err(|error| unsupported(format!("open git repository: {error}")))?;
        Ok(Self {
            repository_path: repository_path.to_path_buf(),
        })
    }

    /// Returns the repository path backing this transport.
    pub fn repository_path(&self) -> &Path {
        &self.repository_path
    }
}

impl RegistryTransport for Git2Transport {
    fn publish(&self, _envelope: TransportEnvelope<'_>) -> RegistryResult<()> {
        Err(unsupported("git2 publish transport"))
    }

    fn fetch(&self, _key: &str) -> RegistryResult<Option<Vec<u8>>> {
        Err(unsupported("git2 fetch transport"))
    }
}
