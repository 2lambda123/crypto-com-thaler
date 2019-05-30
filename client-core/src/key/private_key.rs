use failure::ResultExt;
use parity_codec::{Decode, Encode, Input, Output};
use rand::rngs::OsRng;
use secp256k1::{Message, PublicKey as SecpPublicKey, SecretKey};
use zeroize::Zeroize;

use chain_core::tx::witness::TxInWitness;
use client_common::{ErrorKind, Result};

use crate::{PublicKey, SECP};

/// Private key used in Crypto.com Chain
#[derive(Debug, PartialEq, Clone)]
pub struct PrivateKey(SecretKey);

impl PrivateKey {
    /// Generates a new private key
    pub fn new() -> Result<PrivateKey> {
        let mut rng = OsRng::new().context(ErrorKind::KeyGenerationError)?;
        let secret_key = SecretKey::new(&mut rng);

        Ok(PrivateKey(secret_key))
    }

    /// Serializes current private key
    pub fn serialize(&self) -> Vec<u8> {
        self.0[..].to_vec()
    }

    /// Deserializes private key from bytes
    pub fn deserialize_from(bytes: &[u8]) -> Result<PrivateKey> {
        let secret_key: SecretKey =
            SecretKey::from_slice(bytes).context(ErrorKind::DeserializationError)?;

        Ok(PrivateKey(secret_key))
    }

    /// Signs a message with current private key
    pub fn sign<T: AsRef<[u8]>>(&self, bytes: T) -> Result<TxInWitness> {
        let message =
            Message::from_slice(bytes.as_ref()).context(ErrorKind::DeserializationError)?;
        let signature = SECP.with(|secp| secp.sign_recoverable(&message, &self.0));
        Ok(TxInWitness::BasicRedeem(signature))
    }
}

impl Encode for PrivateKey {
    fn encode_to<W: Output>(&self, dest: &mut W) {
        self.serialize().encode_to(dest)
    }
}

impl Decode for PrivateKey {
    fn decode<I: Input>(input: &mut I) -> Option<Self> {
        match <Vec<u8>>::decode(input) {
            None => None,
            Some(serialized) => match PrivateKey::deserialize_from(&serialized) {
                Err(_) => None,
                Ok(private_key) => Some(private_key),
            },
        }
    }
}

impl From<&PrivateKey> for PublicKey {
    fn from(private_key: &PrivateKey) -> Self {
        let secret_key = &private_key.0;

        let public_key = SECP.with(|secp| SecpPublicKey::from_secret_key(secp, secret_key));

        public_key.into()
    }
}

impl From<&PrivateKey> for SecretKey {
    fn from(private_key: &PrivateKey) -> Self {
        private_key.0.clone()
    }
}

impl Zeroize for PrivateKey {
    fn zeroize(&mut self) {
        self.0.zeroize()
    }
}

impl Drop for PrivateKey {
    fn drop(&mut self) {
        self.zeroize()
    }
}

#[cfg(test)]
mod tests {
    use super::PrivateKey;

    #[test]
    fn check_serialization() {
        // Hex representation: "c553a03604235df8fcd14fc6d1e5b18a219fbcc6e93effcfcf768e2977a74ec2"
        let secret_arr: Vec<u8> = vec![
            197, 83, 160, 54, 4, 35, 93, 248, 252, 209, 79, 198, 209, 229, 177, 138, 33, 159, 188,
            198, 233, 62, 255, 207, 207, 118, 142, 41, 119, 167, 78, 194,
        ];

        let private_key = PrivateKey::deserialize_from(&secret_arr)
            .expect("Unable to deserialize private key from byte array");

        let private_arr = private_key.serialize();

        assert_eq!(
            secret_arr, private_arr,
            "Serialization / Deserialization is implemented incorrectly"
        );
    }

    #[test]
    fn check_rng_serialization() {
        let private_key = PrivateKey::new().expect("Unable to generate private key");

        let private_arr = private_key.serialize();

        let secret_key =
            PrivateKey::deserialize_from(&private_arr).expect("Unable to deserialize private key");

        assert_eq!(
            private_key, secret_key,
            "Serialization / Deserialization is implemented incorrectly"
        );
    }
}
