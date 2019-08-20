use jsonrpc_core::Result;
use jsonrpc_derive::rpc;

use crate::rpc::websocket_rpc::{WalletInfo, WalletInfos, WebsocketRpc};
use crate::server::{to_rpc_error, WalletRequest};
use chain_core::state::account::StakedStateAddress;
use client_common::tendermint::Client;
use client_common::{Error, ErrorKind, PrivateKey, PublicKey, Storage};
use client_core::{MultiSigWalletClient, WalletClient};
use client_index::synchronizer::ManualSynchronizer;
use client_index::BlockHandler;
use serde_json::json;

#[rpc]
pub trait SyncRpc: Send + Sync {
    #[rpc(name = "sync")]
    fn sync(&self, request: WalletRequest) -> Result<()>;

    #[rpc(name = "sync_all")]
    fn sync_all(&self, request: WalletRequest) -> Result<()>;

    #[rpc(name = "sync_continuous")]
    fn sync_continuous(&self, request: WalletRequest) -> Result<String>;
}

pub struct SyncRpcImpl<T, S, C, H>
where
    T: WalletClient,
    S: Storage,
    C: Client,
    H: BlockHandler,
{
    client: T,
    synchronizer: ManualSynchronizer<S, C, H>,
}

impl<T, S, C, H> SyncRpc for SyncRpcImpl<T, S, C, H>
where
    T: WalletClient + MultiSigWalletClient + 'static,
    S: Storage + 'static,
    C: Client + 'static,
    H: BlockHandler + 'static,
{
    fn sync(&self, request: WalletRequest) -> Result<()> {
        let (view_key, private_key, staking_addresses) =
            self.prepare_synchronized_parameters(&request)?;

        self.synchronizer
            .sync(&staking_addresses, &view_key, &private_key)
            .map_err(to_rpc_error)
    }

    fn sync_all(&self, request: WalletRequest) -> Result<()> {
        let (view_key, private_key, staking_addresses) =
            self.prepare_synchronized_parameters(&request)?;

        self.synchronizer
            .sync_all(&staking_addresses, &view_key, &private_key)
            .map_err(to_rpc_error)
    }

    fn sync_continuous(&self, request: WalletRequest) -> Result<String> {
        let view_key = self
            .client
            .view_key(&request.name, &request.passphrase)
            .map_err(to_rpc_error)?;
        let private_key = self
            .client
            .private_key(&request.passphrase, &view_key)
            .map_err(to_rpc_error)?
            .ok_or_else(|| Error::from(ErrorKind::WalletNotFound))
            .map_err(to_rpc_error)?;

        let staking_addresses = self
            .client
            .staking_addresses(&request.name, &request.passphrase)
            .map_err(to_rpc_error)?;

        let info = WalletInfo {
            name: request.name.to_string(),
            staking_addresses,
            view_key:view_key.clone(),
            private_key,
        };

        let ret = view_key.to_string();
        Ok(ret)
    }
}

impl<T, S, C, H> SyncRpcImpl<T, S, C, H>
where
    T: WalletClient,
    S: Storage,
    C: Client,
    H: BlockHandler,
{
    pub fn new(client: T, synchronizer: ManualSynchronizer<S, C, H>) -> Self {
        SyncRpcImpl {
            client,
            synchronizer,
        }
    }

    fn prepare_synchronized_parameters(
        &self,
        request: &WalletRequest,
    ) -> Result<(PublicKey, PrivateKey, Vec<StakedStateAddress>)> {
        let view_key = self
            .client
            .view_key(&request.name, &request.passphrase)
            .map_err(to_rpc_error)?;
        let private_key = self
            .client
            .private_key(&request.passphrase, &view_key)
            .map_err(to_rpc_error)?
            .ok_or_else(|| Error::from(ErrorKind::WalletNotFound))
            .map_err(to_rpc_error)?;

        let staking_addresses = self
            .client
            .staking_addresses(&request.name, &request.passphrase)
            .map_err(to_rpc_error)?;

        Ok((view_key, private_key, staking_addresses))
    }
}
