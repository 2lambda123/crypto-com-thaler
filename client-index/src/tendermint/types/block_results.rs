#![allow(missing_docs)]

use base64::decode;
use failure::ResultExt;
use serde::Deserialize;

use chain_core::tx::data::TxId;
use client_common::{ErrorKind, Result};

#[derive(Debug, Deserialize)]
pub struct BlockResults {
    height: String,
    results: Results,
}

#[derive(Debug, Deserialize)]
struct Results {
    #[serde(rename = "DeliverTx")]
    deliver_tx: Vec<DeliverTx>,
}

#[derive(Debug, Deserialize)]
struct DeliverTx {
    tags: Vec<Tag>,
}

#[derive(Debug, Deserialize)]
struct Tag {
    key: String,
    value: String,
}

impl BlockResults {
    /// Returns valid transaction ids in block results
    pub fn ids(&self) -> Result<Vec<TxId>> {
        let mut transactions: Vec<TxId> = Default::default();

        for transaction in self.results.deliver_tx.iter() {
            for tag in transaction.tags.iter() {
                let decoded = decode(&tag.value).context(ErrorKind::DeserializationError)?;
                if 32 != decoded.len() {
                    return Err(ErrorKind::DeserializationError.into());
                }

                let mut id: [u8; 32] = [0; 32];
                id.copy_from_slice(&decoded);

                transactions.push(id.into());
            }
        }

        Ok(transactions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_ids() {
        let block_results = BlockResults {
            height: "2".to_owned(),
            results: Results {
                deliver_tx: vec![DeliverTx {
                    tags: vec![Tag {
                        key: "dHhpZA==".to_owned(),
                        value: "kOzcmhZgAAaw5roBdqDNniwRjjKNe+foJEiDAOObTDQ=".to_owned(),
                    }],
                }],
            },
        };

        assert_eq!(1, block_results.ids().unwrap().len());
    }

    #[test]
    fn check_wrong_id() {
        let block_results = BlockResults {
            height: "2".to_owned(),
            results: Results {
                deliver_tx: vec![DeliverTx {
                    tags: vec![Tag {
                        key: "dHhpZA==".to_owned(),
                        value: "kOzcmhZgAAaw5riwRjjKNe+foJEiDAOObTDQ=".to_owned(),
                    }],
                }],
            },
        };

        assert!(block_results.ids().is_err());
    }
}
