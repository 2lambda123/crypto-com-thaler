#![cfg_attr(all(feature = "mesalock_sgx", not(target_env = "sgx")), no_std)]
#![cfg_attr(
    all(target_env = "sgx", target_vendor = "mesalock"),
    feature(rustc_private)
)]

#[cfg(all(feature = "mesalock_sgx", not(target_env = "sgx")))]
#[macro_use]
extern crate sgx_tstd as std;
mod filter;
use chain_core::state::account::StakedStateAddress;
use filter::Bloom;
use parity_scale_codec::Encode;
use secp256k1::key::PublicKey;
use std::convert::TryFrom;

/// Probabilistic fixed-size filter wrapper
#[derive(Default, Debug)]
pub struct BlockFilter {
    // may be replaced with GCS, e.g. https://github.com/dac-gmbh/golomb-set
    bloom: Bloom,
    modified: bool,
}

impl BlockFilter {
    /// adds a view key to the filter
    pub fn add_view_key(&mut self, view_key: &PublicKey) {
        self.modified = true;
        self.bloom.set(&view_key.serialize()[..]);
    }

    /// adds a staked state address to the filter
    /// FIXME: to be deprecated/removed -- just use events in ABCI and regular Tendermint indexing
    pub fn add_staked_state_address(&mut self, address: &StakedStateAddress) {
        self.modified = true;
        self.bloom.set(&address.encode());
    }

    /// gets a Key-Value payload for tendermint events (if any view keys were added)
    pub fn get_tendermint_kv(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        if self.modified {
            Some((Vec::from(&b"ethbloom"[..]), self.bloom.data()))
        } else {
            None
        }
    }

    /// tests if a view key is in the filter
    /// true = maybe present
    /// false = not present
    pub fn check_view_key(&self, view_key: &PublicKey) -> bool {
        self.bloom.check(&view_key.serialize())
    }

    /// tests if a staked state address is in the filter
    /// true = maybe present
    /// false = not present
    /// FIXME: to be deprecated/removed -- just use events in ABCI and regular Tendermint indexing
    pub fn check_staked_state_address(&self, address: &StakedStateAddress) -> bool {
        self.bloom.check(&address.encode())
    }

    /// check if view keys were added since its creation
    pub fn is_modified(&self) -> bool {
        self.modified
    }
}

impl TryFrom<&[u8]> for BlockFilter {
    type Error = &'static str;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if value.len() != 256 {
            Err("Invalid length, ethbloom is expected to be 256-bytes")
        } else {
            let mut bloom_array = [0u8; 256];
            bloom_array.copy_from_slice(&value);
            let bloom = Bloom::from(&bloom_array);
            Ok(BlockFilter {
                bloom,
                modified: false,
            })
        }
    }
}
