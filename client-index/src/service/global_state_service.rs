use bincode::{deserialize, serialize};
use failure::ResultExt;

use client_common::{ErrorKind, Result, Storage};

const KEYSPACE: &str = "index_global_state";
const LAST_BLOCK_HEIGHT: &str = "last_block_height";

/// Exposes functionalities for managing client's global state
pub struct GlobalStateService<S> {
    storage: S,
}

impl<S> GlobalStateService<S>
where
    S: Storage,
{
    /// Creates new instance of global state service
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    /// Returns currently stored last block height
    pub fn last_block_height(&self) -> Result<Option<u64>> {
        let bytes = self.storage.get(KEYSPACE, LAST_BLOCK_HEIGHT)?;

        match bytes {
            None => Ok(None),
            Some(bytes) => {
                let last_block_height: u64 =
                    deserialize(&bytes).context(ErrorKind::DeserializationError)?;
                Ok(Some(last_block_height))
            }
        }
    }

    /// Updates last block height with given value and returns old value
    pub fn set_last_block_height(&self, last_block_height: u64) -> Result<Option<u64>> {
        let bytes = serialize(&last_block_height).context(ErrorKind::SerializationError)?;

        let old_bytes = self.storage.set(KEYSPACE, LAST_BLOCK_HEIGHT, bytes)?;

        match old_bytes {
            None => Ok(None),
            Some(bytes) => {
                let last_block_height: u64 =
                    deserialize(&bytes).context(ErrorKind::DeserializationError)?;
                Ok(Some(last_block_height))
            }
        }
    }

    /// Synchronizes last block height with Crypto.com Chain and returns old (if exists) and new value
    // pub fn sync_last_block_height(&self) -> Result<(Option<u64>, u64)> {
    //     let status = self.client.status()?;
    //     let last_block_height = status.last_block_height()?;

    //     let old_last_block_height = self.set_last_block_height(last_block_height)?;

    //     Ok((old_last_block_height, last_block_height))
    // }

    /// Clears all storage
    pub fn clear(&self) -> Result<()> {
        self.storage.clear(KEYSPACE)
    }
}
