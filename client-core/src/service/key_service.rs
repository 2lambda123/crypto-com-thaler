use bincode::{deserialize, serialize};
use failure::ResultExt;
use secstr::SecStr;

use client_common::{ErrorKind, Result, SecureStorage, Storage};

use crate::PrivateKey;

const KEYSPACE: &str = "core_key";

/// Exposes functionality for managing public and private keys
#[derive(Default, Clone)]
pub struct KeyService<T: Storage> {
    storage: T,
}

impl<T> KeyService<T>
where
    T: Storage,
{
    /// Creates a new instance of key service
    pub fn new(storage: T) -> Self {
        KeyService { storage }
    }

    /// Generates a new address for given wallet ID
    pub fn generate(&self, wallet_id: &str, passphrase: &SecStr) -> Result<PrivateKey> {
        let private_key = PrivateKey::new()?;

        let private_keys = self.storage.get_secure(KEYSPACE, wallet_id, passphrase)?;

        let mut private_keys = match private_keys {
            None => Vec::new(),
            Some(private_keys) => {
                deserialize(&private_keys).context(ErrorKind::DeserializationError)?
            }
        };

        private_keys.push(private_key.serialize()?);

        self.storage.set_secure(
            KEYSPACE,
            wallet_id,
            serialize(&private_keys).context(ErrorKind::SerializationError)?,
            passphrase,
        )?;

        Ok(private_key)
    }

    /// Returns all the keys stored for given wallet ID
    pub fn get_keys(
        &self,
        wallet_id: &str,
        passphrase: &SecStr,
    ) -> Result<Option<Vec<PrivateKey>>> {
        let private_keys = self.storage.get_secure(KEYSPACE, wallet_id, passphrase)?;

        match private_keys {
            None => Ok(None),
            Some(bytes) => {
                let private_keys: Vec<Vec<u8>> =
                    deserialize(&bytes).context(ErrorKind::DeserializationError)?;

                let private_keys = private_keys
                    .iter()
                    .map(|inner| -> Result<PrivateKey> { PrivateKey::deserialize_from(inner) })
                    .collect::<Result<Vec<PrivateKey>>>()?;

                Ok(Some(private_keys))
            }
        }
    }

    /// Clears all storage
    pub fn clear(&self) -> Result<()> {
        self.storage.clear(KEYSPACE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use client_common::storage::MemoryStorage;

    #[test]
    fn check_flow() {
        let key_service = KeyService::new(MemoryStorage::default());

        let private_key = key_service
            .generate("wallet_id", &SecStr::from("passphrase"))
            .expect("Unable to generate private key");

        let new_private_key = key_service
            .generate("wallet_id", &SecStr::from("passphrase"))
            .expect("Unable to generate private key");

        let keys = key_service
            .get_keys("wallet_id", &SecStr::from("passphrase"))
            .expect("Unable to get keys from storage")
            .expect("No keys found");

        assert_eq!(2, keys.len(), "Unexpected key length");
        assert_eq!(private_key, keys[0], "Invalid private key found");
        assert_eq!(new_private_key, keys[1], "Invalid private key found");

        let error = key_service
            .get_keys("wallet_id", &SecStr::from("incorrect_passphrase"))
            .expect_err("Decryption worked with incorrect passphrase");

        assert_eq!(
            error.kind(),
            ErrorKind::DecryptionError,
            "Invalid error kind"
        );

        assert!(key_service.clear().is_ok());

        assert!(key_service
            .get_keys("wallet_id", &SecStr::from("passphrase"))
            .unwrap()
            .is_none());
    }
}
