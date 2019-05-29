use std::cmp::Ordering;
use std::fmt;

use parity_codec::{Decode, Encode, Input, Output};

use crate::common::{H264, H512};

/// there's no [T; 33] / [u8; 33] impl in parity-codec :/
#[derive(Clone)]
pub struct RawPubkey(H264);

impl From<H264> for RawPubkey {
    fn from(h: H264) -> Self {
        RawPubkey(h)
    }
}

impl AsRef<[u8]> for RawPubkey {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl PartialEq for RawPubkey {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for RawPubkey {}

impl PartialOrd for RawPubkey {
    #[inline]
    fn partial_cmp(&self, other: &RawPubkey) -> Option<Ordering> {
        PartialOrd::partial_cmp(&&self.0[..], &&other.0[..])
    }
    #[inline]
    fn lt(&self, other: &RawPubkey) -> bool {
        PartialOrd::lt(&&self.0[..], &&other.0[..])
    }
    #[inline]
    fn le(&self, other: &RawPubkey) -> bool {
        PartialOrd::le(&&self.0[..], &&other.0[..])
    }
    #[inline]
    fn ge(&self, other: &RawPubkey) -> bool {
        PartialOrd::ge(&&self.0[..], &&other.0[..])
    }
    #[inline]
    fn gt(&self, other: &RawPubkey) -> bool {
        PartialOrd::gt(&&self.0[..], &&other.0[..])
    }
}

impl Ord for RawPubkey {
    #[inline]
    fn cmp(&self, other: &RawPubkey) -> Ordering {
        Ord::cmp(&&self.0[..], &&other.0[..])
    }
}

impl fmt::Debug for RawPubkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&&self.0[..], f)
    }
}

impl RawPubkey {
    /// Extracts a byte slice containing the entire public key.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Encode for RawPubkey {
    fn encode_to<W: Output>(&self, dest: &mut W) {
        for item in self.0.iter() {
            dest.push_byte(*item);
        }
    }
}

impl Decode for RawPubkey {
    fn decode<I: Input>(input: &mut I) -> Option<Self> {
        let mut r = [0u8; 33];
        for item in (&mut r).iter_mut() {
            *item = input.read_byte()?;
        }
        Some(RawPubkey(r))
    }
}

pub type RawSignature = H512;
