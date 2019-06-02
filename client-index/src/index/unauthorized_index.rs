use chain_core::tx::data::address::ExtendedAddr;
use chain_core::tx::data::input::TxoPointer;
use chain_core::tx::data::output::TxOut;
use chain_core::tx::data::{Tx, TxId};
use client_common::balance::TransactionChange;
use client_common::serializable::SerializableCoin;
use client_common::{ErrorKind, Result};

use crate::Index;

/// `Index` which returns `PermissionDenied` error for each function call.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnauthorizedIndex;

impl Index for UnauthorizedIndex {
    fn sync(&self) -> Result<()> {
        Err(ErrorKind::PermissionDenied.into())
    }

    fn sync_all(&self) -> Result<()> {
        Err(ErrorKind::PermissionDenied.into())
    }

    fn transaction_changes(&self, _address: &ExtendedAddr) -> Result<Vec<TransactionChange>> {
        Err(ErrorKind::PermissionDenied.into())
    }

    fn balance(&self, _address: &ExtendedAddr) -> Result<SerializableCoin> {
        Err(ErrorKind::PermissionDenied.into())
    }

    fn unspent_transactions(&self, _address: &ExtendedAddr) -> Result<Vec<(TxoPointer, SerializableCoin)>> {
        Err(ErrorKind::PermissionDenied.into())
    }

    fn transaction(&self, _id: &TxId) -> Result<Option<Tx>> {
        Err(ErrorKind::PermissionDenied.into())
    }

    fn output(&self, _id: &TxId, _index: usize) -> Result<TxOut> {
        Err(ErrorKind::PermissionDenied.into())
    }

    fn broadcast_transaction(&self, _transaction: &[u8]) -> Result<()> {
        Err(ErrorKind::PermissionDenied.into())
    }
}
