#![allow(missing_docs)]

use base64::decode;
use chrono::offset::Utc;
use chrono::DateTime;
use failure::ResultExt;
use serde::Deserialize;

use chain_core::tx::TxAux;
use client_common::{ErrorKind, Result};

#[derive(Debug, Deserialize)]
pub struct Block {
    pub block: BlockInner,
}

#[derive(Debug, Deserialize)]
pub struct BlockInner {
    pub header: Header,
    pub data: Data,
}

#[derive(Debug, Deserialize)]
pub struct Data {
    pub txs: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Header {
    pub height: String,
    pub time: DateTime<Utc>,
}

impl Block {
    /// Returns transactions in a block (this may also contain invalid transactions)
    pub fn transactions(&self) -> Result<Vec<TxAux>> {
        self.block
            .data
            .txs
            .iter()
            .map(|raw_tx| Ok(decode(&raw_tx).context(ErrorKind::DeserializationError)?))
            .map(|bytes: Result<Vec<u8>>| {
                Ok(rlp::decode(&bytes?).context(ErrorKind::DeserializationError)?)
            })
            .collect::<Result<Vec<TxAux>>>()
    }

    /// Returns height of this block
    pub fn height(&self) -> Result<u64> {
        Ok(self
            .block
            .header
            .height
            .parse::<u64>()
            .context(ErrorKind::DeserializationError)?)
    }

    /// Returns time of this block
    pub fn time(&self) -> DateTime<Utc> {
        self.block.header.time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::str::FromStr;

    #[test]
    fn check_transactions() {
        let block = Block {
            block: BlockInner {
                header: Header {
                    height: "1".to_owned(),
                    time: DateTime::from_str("2019-04-09T09:38:41.735577Z").unwrap(),
                },
                data: Data {
                    txs: vec!["+JWA+Erj4qBySKi4J+krjuZi++QuAnQITDv9YzjXV0RcDuk+S7pMeIDh4NaAlHkGYaL9naP+5TyquAhZ7K4SWiCliAAA6IkEI8eKw4GrwPhG+ESAAbhASZdu2rJI4Et7q93KedoEsTVFUOCPt8nyY0pGOqixhI4TvORYPVFmJiG+Lsr6L1wmwBLIwxJenWTyKZ8rKrwfkg==".to_owned()]
                }
            }
        };
        assert_eq!(1, block.transactions().unwrap().len());
    }

    #[test]
    fn check_wrong_transaction() {
        let block = Block {
            block: BlockInner {
                header: Header {
                    height: "1".to_owned(),
                    time: DateTime::from_str("2019-04-09T09:38:41.735577Z").unwrap(),
                },
                data: Data {
                    txs: vec!["+JWA+Erj4qBySKi4J+krjuZi++QuAnQITDv9YzjXV0RcDuk+S7pMeIDh4NaA4SWiCliAAA6IkEI8eKw4GrwPhG+ESAAbhASZdu2rJI4Et7q93KedoEsTVFUOCPt8nyY0pGOqixhI4TvORYPVFmJiG+Lsr6L1wmwBLIwxJenWTyKZ8rKrwfkg==".to_owned()]
                }
            }
        };

        assert!(block.transactions().is_err());
    }

    #[test]
    fn check_height() {
        let block = Block {
            block: BlockInner {
                header: Header {
                    height: "1".to_owned(),
                    time: DateTime::from_str("2019-04-09T09:38:41.735577Z").unwrap(),
                },
                data: Data {
                    txs: vec!["+JWA+Erj4qBySKi4J+krjuZi++QuAnQITDv9YzjXV0RcDuk+S7pMeIDh4NaAlHkGYaL9naP+5TyquAhZ7K4SWiCliAAA6IkEI8eKw4GrwPhG+ESAAbhASZdu2rJI4Et7q93KedoEsTVFUOCPt8nyY0pGOqixhI4TvORYPVFmJiG+Lsr6L1wmwBLIwxJenWTyKZ8rKrwfkg==".to_owned()]
                }
            }
        };

        assert_eq!(1, block.height().unwrap());
    }

    #[test]
    fn check_wrong_height() {
        let block = Block {
            block: BlockInner {
                header: Header {
                    height: "a".to_owned(),
                    time: DateTime::from_str("2019-04-09T09:38:41.735577Z").unwrap(),
                },
                data: Data {
                    txs: vec!["+JWA+Erj4qBySKi4J+krjuZi++QuAnQITDv9YzjXV0RcDuk+S7pMeIDh4NaAlHkGYaL9naP+5TyquAhZ7K4SWiCliAAA6IkEI8eKw4GrwPhG+ESAAbhASZdu2rJI4Et7q93KedoEsTVFUOCPt8nyY0pGOqixhI4TvORYPVFmJiG+Lsr6L1wmwBLIwxJenWTyKZ8rKrwfkg==".to_owned()]
                }
            }
        };

        assert!(block.height().is_err());
    }
}
