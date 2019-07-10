use jsonrpc_core::Result;
use jsonrpc_derive::rpc;
use secstr::SecUtf8;
use serde::{Deserialize, Serialize};

use chain_core::init::coin::Coin;
use chain_core::tx::data::address::ExtendedAddr;
use chain_core::tx::data::attribute::TxAttributes;
use chain_core::tx::data::output::TxOut;
use client_common::balance::TransactionChange;
use client_core::wallet::WalletClient;

use crate::server::{rpc_error_from_string, to_rpc_error};

#[rpc]
pub trait WalletRpc {
    #[rpc(name = "wallet_addresses")]
    fn addresses(&self, request: WalletRequest) -> Result<Vec<String>>;

    #[rpc(name = "wallet_balance")]
    fn balance(&self, request: WalletRequest) -> Result<Coin>;

    #[rpc(name = "wallet_create")]
    fn create(&self, request: WalletRequest) -> Result<String>;

    #[rpc(name = "wallet_list")]
    fn list(&self) -> Result<Vec<String>>;

    #[rpc(name = "wallet_sendtoaddress")]
    fn sendtoaddress(&self, request: WalletRequest, to_address: String, amount: u64) -> Result<()>;

    #[rpc(name = "sync")]
    fn sync(&self) -> Result<()>;

    #[rpc(name = "sync_all")]
    fn sync_all(&self) -> Result<()>;

    #[rpc(name = "wallet_transactions")]
    fn transactions(&self, request: WalletRequest) -> Result<Vec<TransactionChange>>;
}

pub struct WalletRpcImpl<T: WalletClient + Send + Sync> {
    client: T,
    chain_id: u8,
}

impl<T> WalletRpcImpl<T>
where
    T: WalletClient + Send + Sync,
{
    pub fn new(client: T, chain_id: u8) -> Self {
        WalletRpcImpl { client, chain_id }
    }
}

impl<T> WalletRpc for WalletRpcImpl<T>
where
    T: WalletClient + Send + Sync + 'static,
{
    fn addresses(&self, request: WalletRequest) -> Result<Vec<String>> {
        // TODO: Currently, it only returns staking addresses
        match self
            .client
            .staking_addresses(&request.name, &request.passphrase)
        {
            Ok(addresses) => addresses
                .iter()
                .map(|address| Ok(address.to_string()))
                .collect(),
            Err(e) => Err(to_rpc_error(e)),
        }
    }

    fn balance(&self, request: WalletRequest) -> Result<Coin> {
        self.sync()?;

        match self.client.balance(&request.name, &request.passphrase) {
            Ok(balance) => Ok(balance),
            Err(e) => Err(to_rpc_error(e)),
        }
    }

    fn create(&self, request: WalletRequest) -> Result<String> {
        if let Err(e) = self.client.new_wallet(&request.name, &request.passphrase) {
            return Err(to_rpc_error(e));
        }

        if let Err(e) = self
            .client
            .new_single_transfer_address(&request.name, &request.passphrase)
        {
            Err(to_rpc_error(e))
        } else {
            Ok(request.name)
        }
    }

    fn list(&self) -> Result<Vec<String>> {
        match self.client.wallets() {
            Ok(wallets) => Ok(wallets),
            Err(e) => Err(to_rpc_error(e)),
        }
    }

    fn sendtoaddress(&self, request: WalletRequest, to_address: String, amount: u64) -> Result<()> {
        self.sync()?;

        let address = to_address
            .parse::<ExtendedAddr>()
            .map_err(|err| rpc_error_from_string(format!("{}", err)))?;
        let coin = Coin::new(amount).map_err(|err| rpc_error_from_string(format!("{}", err)))?;
        let tx_out = TxOut::new(address, coin);
        let tx_attributes = TxAttributes::new(self.chain_id);

        let return_address = self
            .client
            .new_single_transfer_address(&request.name, &request.passphrase)
            .map_err(to_rpc_error)?;

        let transaction = self
            .client
            .create_transaction(
                &request.name,
                &request.passphrase,
                vec![tx_out],
                tx_attributes,
                None,
                return_address,
            )
            .map_err(to_rpc_error)?;

        self.client
            .broadcast_transaction(&transaction)
            .map_err(to_rpc_error)
    }

    fn sync(&self) -> Result<()> {
        if let Err(e) = self.client.sync() {
            Err(to_rpc_error(e))
        } else {
            Ok(())
        }
    }

    fn sync_all(&self) -> Result<()> {
        if let Err(e) = self.client.sync_all() {
            Err(to_rpc_error(e))
        } else {
            Ok(())
        }
    }

    fn transactions(&self, request: WalletRequest) -> Result<Vec<TransactionChange>> {
        self.sync()?;

        match self.client.history(&request.name, &request.passphrase) {
            Ok(transaction_change) => Ok(transaction_change),
            Err(e) => Err(to_rpc_error(e)),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WalletRequest {
    name: String,
    passphrase: SecUtf8,
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::DateTime;
    use std::time::SystemTime;

    use chain_core::init::coin::CoinError;
    use chain_core::tx::data::input::TxoPointer;
    use chain_core::tx::data::{Tx, TxId};
    use chain_core::tx::fee::{Fee, FeeAlgorithm};
    use chain_core::tx::TxAux;
    use client_common::balance::BalanceChange;
    use client_common::storage::MemoryStorage;
    use client_common::{Error, ErrorKind, Result as CommonResult, Transaction};
    use client_core::signer::DefaultSigner;
    use client_core::transaction_builder::DefaultTransactionBuilder;
    use client_core::wallet::DefaultWalletClient;
    use client_index::Index;

    #[derive(Default)]
    pub struct MockIndex;

    impl Index for MockIndex {
        fn sync(&self) -> CommonResult<()> {
            Ok(())
        }

        fn sync_all(&self) -> CommonResult<()> {
            Ok(())
        }

        fn transaction_changes(
            &self,
            address: &ExtendedAddr,
        ) -> CommonResult<Vec<TransactionChange>> {
            Ok(vec![TransactionChange {
                transaction_id: [0u8; 32],
                address: address.clone(),
                balance_change: BalanceChange::Incoming(Coin::new(30).unwrap()),
                height: 1,
                time: DateTime::from(SystemTime::now()),
            }])
        }

        fn balance(&self, _: &ExtendedAddr) -> CommonResult<Coin> {
            Ok(Coin::new(30).unwrap())
        }

        fn unspent_transactions(
            &self,
            _address: &ExtendedAddr,
        ) -> CommonResult<Vec<(TxoPointer, TxOut)>> {
            Ok(Vec::new())
        }

        fn transaction(&self, _: &TxId) -> CommonResult<Option<Transaction>> {
            Ok(Some(Transaction::TransferTransaction(Tx {
                inputs: vec![TxoPointer {
                    id: [0u8; 32],
                    index: 1,
                }],
                outputs: Default::default(),
                attributes: TxAttributes::new(171),
            })))
        }

        fn output(&self, _id: &TxId, _index: usize) -> CommonResult<TxOut> {
            Ok(TxOut {
                address: ExtendedAddr::OrTree([0; 32]),
                value: Coin::new(10000000000000000000).unwrap(),
                valid_from: None,
            })
        }

        fn broadcast_transaction(&self, _transaction: &[u8]) -> CommonResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct ZeroFeeAlgorithm;

    impl FeeAlgorithm for ZeroFeeAlgorithm {
        fn calculate_fee(&self, _num_bytes: usize) -> std::result::Result<Fee, CoinError> {
            Ok(Fee::new(Coin::zero()))
        }

        fn calculate_for_txaux(&self, _txaux: &TxAux) -> std::result::Result<Fee, CoinError> {
            Ok(Fee::new(Coin::zero()))
        }
    }

    #[test]
    fn test_create_duplicated_wallet() {
        let wallet_rpc = setup_wallet_rpc();

        assert_eq!(
            "Default".to_owned(),
            wallet_rpc
                .create(create_wallet_request("Default", "123456"))
                .unwrap()
        );

        assert_eq!(
            to_rpc_error(Error::from(ErrorKind::AlreadyExists)),
            wallet_rpc
                .create(create_wallet_request("Default", "123456"))
                .unwrap_err()
        );
    }

    #[test]
    fn test_create_and_list_wallet_flow() {
        let wallet_rpc = setup_wallet_rpc();

        assert_eq!(0, wallet_rpc.list().unwrap().len());

        assert_eq!(
            "Default".to_owned(),
            wallet_rpc
                .create(create_wallet_request("Default", "123456"))
                .unwrap()
        );

        assert_eq!(vec!["Default"], wallet_rpc.list().unwrap());

        assert_eq!(
            "Personal".to_owned(),
            wallet_rpc
                .create(create_wallet_request("Personal", "123456"))
                .unwrap()
        );

        let wallet_list = wallet_rpc.list().unwrap();
        assert_eq!(2, wallet_list.len());
        assert!(wallet_list.contains(&"Default".to_owned()));
        assert!(wallet_list.contains(&"Personal".to_owned()));
    }

    #[test]
    fn test_create_and_list_wallet_addresses_flow() {
        let wallet_rpc = setup_wallet_rpc();

        assert_eq!(
            to_rpc_error(Error::from(ErrorKind::WalletNotFound)),
            wallet_rpc
                .addresses(create_wallet_request("Default", "123456"))
                .unwrap_err()
        );

        assert_eq!(
            "Default".to_owned(),
            wallet_rpc
                .create(create_wallet_request("Default", "123456"))
                .unwrap()
        );

        assert_eq!(
            1,
            wallet_rpc
                .addresses(create_wallet_request("Default", "123456"))
                .unwrap()
                .len()
        );
    }

    #[test]
    fn test_wallet_balance() {
        let wallet_rpc = setup_wallet_rpc();

        wallet_rpc
            .create(create_wallet_request("Default", "123456"))
            .unwrap();
        assert_eq!(
            Coin::new(30).unwrap(),
            wallet_rpc
                .balance(create_wallet_request("Default", "123456"))
                .unwrap()
        )
    }

    #[test]
    fn test_wallet_transactions() {
        let wallet_rpc = setup_wallet_rpc();

        wallet_rpc
            .create(create_wallet_request("Default", "123456"))
            .unwrap();
        assert_eq!(
            1,
            wallet_rpc
                .transactions(create_wallet_request("Default", "123456"))
                .unwrap()
                .len()
        )
    }

    fn setup_wallet_rpc() -> WalletRpcImpl<
        DefaultWalletClient<
            MemoryStorage,
            MockIndex,
            DefaultTransactionBuilder<DefaultSigner<MemoryStorage>, ZeroFeeAlgorithm>,
        >,
    > {
        let storage = MemoryStorage::default();
        let signer = DefaultSigner::new(storage.clone());
        let wallet_client = DefaultWalletClient::builder()
            .with_wallet(storage)
            .with_transaction_read(MockIndex::default())
            .with_transaction_write(DefaultTransactionBuilder::new(
                signer,
                ZeroFeeAlgorithm::default(),
            ))
            .build()
            .unwrap();
        let chain_id = 171u8;

        WalletRpcImpl::new(wallet_client, chain_id)
    }

    fn create_wallet_request(name: &str, passphrase: &str) -> WalletRequest {
        WalletRequest {
            name: name.to_owned(),
            passphrase: SecUtf8::from(passphrase),
        }
    }
}
