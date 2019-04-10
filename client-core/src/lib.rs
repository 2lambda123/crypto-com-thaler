#![deny(missing_docs, unsafe_code, unstable_features)]
//! # Crypto.com Chain Client
//!
//! This crate exposes following functionalities for interacting with Crypto.com Chain:
//! - Wallet creation
//! - Address generation
//! - Transaction syncing and storage
//! - Balance tracking
//! - Transaction creation and signing
//!
//! ## Features
//!
//! This crate has features! Here's a list of features you can enable:
//! - Persistent storage and wallet implementation using [`Sled`](https://crates.io/crates/sled)
//!   - Implementation of [`Storage`](crate::Storage) trait using `Sled` embedded database.
//!   - Implementation of [`Wallet`](crate::Wallet) trait using [`SledStorage`](crate::storage::SledStorage)
//!   - Enable with **`"sled"`** feature flag.
//!   - This feature is enabled by **default**.
//!
//! ### Warning
//!
//! This is a work-in-progress crate and is unusable in its current state.
pub mod key;
pub mod service;
#[cfg(test)]
pub mod test;
pub mod wallet;

#[doc(inline)]
pub use key::{PrivateKey, PublicKey};
#[doc(inline)]
pub use wallet::Wallet;

use secp256k1::{All, Secp256k1};

thread_local! { pub(crate) static SECP: Secp256k1<All> = Secp256k1::new(); }
