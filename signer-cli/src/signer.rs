use failure::Error;
use sled::Db;
use structopt::StructOpt;

use super::commands::AddressCommand;
use super::commands::TransactionCommand;

/// Enum used to specify subcommands under signer
#[derive(Debug, StructOpt)]
#[structopt(
    name = "signer",
    about = "Basic tool for secret management (using enclaves in the future), possibly TX generation and signing"
)]
pub enum Signer {
    /// Used for address management
    #[structopt(name = "address", about = "Address operations")]
    Address {
        #[structopt(subcommand)]
        address_command: AddressCommand,
    },
    /// Used for transaction management
    #[structopt(name = "transaction", about = "Transaction operations")]
    Transaction {
        #[structopt(subcommand)]
        transaction_command: TransactionCommand,
    },
}

impl Signer {
    /// Executes the current command
    pub fn execute(&self, address_storage: &Db) -> Result<(), Error> {
        use Signer::*;

        match self {
            Address { address_command } => address_command.execute(address_storage),
            Transaction {
                transaction_command,
            } => transaction_command.execute(address_storage),
        }
    }
}
