use parity_codec::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::common::H256;
use crate::init::address::{CroAddress, CroAddressError};

use bech32::{u5, Bech32, FromBase32, ToBase32};

/// TODO: opaque types?
type TreeRoot = H256;

/// Currently, only Ethereum-style redeem address + MAST of Or operations (records the root).
/// TODO: HD-addresses?
/// TODO: custom Encode/Decode when data structures are finalized (for backwards/forwards compatibility, encoders/decoders should be able to work with old formats)
#[derive(Debug, PartialEq, Eq, Clone, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ExtendedAddr {
    OrTree(TreeRoot),
}

impl CroAddress<ExtendedAddr> for ExtendedAddr {
    fn to_cro(&self) -> Result<String, CroAddressError> {
        match self {
            ExtendedAddr::OrTree(hash) => {
                let checked_data: Vec<u5> = hash.to_vec().to_base32();
                let encoded =
                    Bech32::new("crmt".into(), checked_data).expect("bech32 crmt encoding");
                Ok(encoded.to_string())
            }
        }
    }

    fn from_cro(encoded: &str) -> Result<Self, CroAddressError> {
        encoded
            .parse::<Bech32>()
            .map_err(|e| CroAddressError::Bech32Error(e))
            .and_then(|a| Vec::from_base32(&a.data()).map_err(|_e| CroAddressError::ConvertError))
            .and_then(|src| {
                let mut a: TreeRoot = [0 as u8; 32];
                a.copy_from_slice(&src.as_slice());
                Ok(ExtendedAddr::OrTree(a))
            })
    }
}

impl fmt::Display for ExtendedAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_cro().unwrap())
    }
}

impl FromStr for ExtendedAddr {
    type Err = CroAddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ExtendedAddr::from_cro(s).map_err(|_e| CroAddressError::ConvertError)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn should_be_correct_textual_address() {
        let a = ExtendedAddr::from_str(
            "0x0e7c045110b8dbf29765047380898919c5cb56f400112233445566778899aabb",
        )
        .unwrap();
        let b = a.to_cro().unwrap();
        assert_eq!(
            b,
            "crmt1pe7qg5gshrdl99m9q3ecpzvfr8zuk4h5qqgjyv6y24n80zye42asr8c7xt"
        );
        let c = ExtendedAddr::from_cro(&b).unwrap();
        assert_eq!(c, a);
    }

}
