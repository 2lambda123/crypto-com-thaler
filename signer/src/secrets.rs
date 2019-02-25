use failure::Error;
use hex::encode;
use rand::rngs::OsRng;
use rand::Rng;
use secp256k1::{
    key::pubkey_combine,
    key::PublicKey,
    key::SecretKey,
    musig::{MuSigSession, MuSigSessionID},
    All, Message, Secp256k1,
};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use structopt::clap::{_clap_count_exprs, arg_enum};
use zeroize::Zeroize;

use chain_core::tx::witness::redeem::EcdsaSignature;
use chain_core::tx::witness::tree::{pk_to_raw, sig_to_raw};
use chain_core::tx::witness::TxInWitness;

arg_enum! {
    /// Different address types
    #[derive(Debug)]
    pub enum AddressType {
        Spend,
        View,
    }
}

/// Struct for specifying secrets
#[derive(Serialize, Deserialize, Debug)]
pub struct Secrets {
    #[serde(skip, default = "Secp256k1::new")]
    secp: Secp256k1<All>,

    spend: SecretKey,
    view: SecretKey,
}

fn random_32_bytes<R: Rng>(rng: &mut R) -> [u8; 32] {
    let mut ret = [0u8; 32];
    rng.fill_bytes(&mut ret);
    ret
}

impl Secrets {
    /// Generates random spend and view secret keys
    pub fn generate() -> Result<Secrets, Error> {
        let mut rand = OsRng::new()?;
        let secp = Secp256k1::new();

        let spend = SecretKey::from_slice(&random_32_bytes(&mut rand))?;

        let view = SecretKey::from_slice(&random_32_bytes(&mut rand))?;

        Ok(Secrets { secp, spend, view })
    }

    /// Returns public key derived from current secret key of given address type
    pub fn get_public_key(&self, address_type: AddressType) -> Result<PublicKey, Error> {
        use AddressType::*;

        match address_type {
            Spend => Ok(PublicKey::from_secret_key(&self.secp, &self.spend)),
            View => Ok(PublicKey::from_secret_key(&self.secp, &self.view)),
        }
    }

    /// Returns address derived from current secret key of given address type
    pub fn get_address(&self, address_type: AddressType) -> Result<String, Error> {
        let public_key = self.get_public_key(address_type)?;

        let mut hasher = Keccak256::new();
        hasher.input(&public_key.serialize()[1..]);
        let hash = hasher.result()[12..].to_vec();

        Ok(encode(hash))
    }

    /// Returns ECDSA signature of message signed with provided secret
    pub fn get_ecdsa_signature(&self, message: &Message) -> Result<TxInWitness, Error> {
        let signature = self.secp.sign_recoverable(message, &self.spend);
        let (recovery_id, serialized_signature) = signature.serialize_compact();

        let r = &serialized_signature[0..32];
        let s = &serialized_signature[32..64];
        let mut sign = EcdsaSignature::default();

        sign.v = recovery_id.to_i32() as u8;
        sign.r.copy_from_slice(r);
        sign.s.copy_from_slice(s);

        Ok(TxInWitness::BasicRedeem(sign))
    }

    /// Returns 2-of-2 (view+spend keys) agg/combined Schnorr signature of message signed with provided secret
    /// NOTE: this method generates the signature from an interactive MuSig protocol session
    /// -- this is not necessary, as both keys are currently generated locally on the same machine,
    /// so it's here more for demonstrative purposes. This will become essential when they are combined from
    /// different devices or different parties. 
    pub fn get_schnorr_signature(&self, message: &Message) -> Result<TxInWitness, Error> {
        use AddressType::*;

        let spend_public_key = self.get_public_key(Spend)?;
        let view_public_key = self.get_public_key(View)?;
        let mut rand = OsRng::new()?;

        let session_id1 = MuSigSessionID::from_slice(&random_32_bytes(&mut rand))?;
        let session_id2 = MuSigSessionID::from_slice(&random_32_bytes(&mut rand))?;

        let (combined_pk, pk_hash) =
            pubkey_combine(&self.secp, &[spend_public_key, view_public_key])?;
        let mut session1 = MuSigSession::new(
            &self.secp,
            session_id1,
            &message,
            &combined_pk,
            &pk_hash,
            2,
            0,
            &self.spend,
        )?;
        let mut session2 = MuSigSession::new(
            &self.secp,
            session_id2,
            &message,
            &combined_pk,
            &pk_hash,
            2,
            1,
            &self.view,
        )?;
        session1.set_nonce_commitment(session2.get_my_nonce_commitment(), 1);
        session2.set_nonce_commitment(session1.get_my_nonce_commitment(), 0);
        let nonces = vec![session1.get_public_nonce()?, session2.get_public_nonce()?];
        for (i, nonce) in nonces.iter().enumerate() {
            session1.set_nonce(i, *nonce)?;
            session2.set_nonce(i, *nonce)?;
        }
        session1.combine_nonces()?;
        session2.combine_nonces()?;
        let partial_sigs = vec![session1.partial_sign()?, session2.partial_sign()?];
        let sig = session1.partial_sig_combine(&partial_sigs)?;

        Ok(TxInWitness::TreeSig(
            pk_to_raw(combined_pk),
            sig_to_raw(sig),
            vec![],
        ))
    }
}

impl Drop for Secrets {
    fn drop(&mut self) {
        // TODO:
        // self.spend.zeroize();
        // self.view.zeroize();
    }
}
