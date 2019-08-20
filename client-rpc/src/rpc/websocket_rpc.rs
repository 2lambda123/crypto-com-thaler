use crate::rpc::websocket_core::WebsocketCore;
use chain_core::state::account::StakedStateAddress;
use client_common::tendermint::Client;
use client_common::{PrivateKey, PublicKey, Storage};
use client_index::BlockHandler;
use futures::future::Future;
use futures::sink::Sink;
use futures::stream::Stream;
use futures::sync::mpsc;
use mpsc::Sender;
use std::thread;
use websocket::result::WebSocketError;
use websocket::{ClientBuilder, OwnedMessage};

pub const CMD_SUBSCRIBE: &str = r#"
    {
        "jsonrpc": "2.0",
        "method": "subscribe",
        "id": "subscribe_reply",
        "params": {
            "query": "tm.event='NewBlock'"
        } 
    }"#;
pub const CMD_BLOCK: &str = r#"
    {
        "method": "block",
        "jsonrpc": "2.0",
        "params": [ "2" ],
        "id": "block_reply"
    }"#;
pub const CMD_STATUS: &str = r#"
    {
        "method": "status",
        "jsonrpc": "2.0",
        "params": [ ],
        "id": "status_reply"
    }"#;

type MyQueue = std::sync::mpsc::Sender<OwnedMessage>;

#[derive(Clone, Debug)]
pub struct WalletInfo {
    pub name: String,
    pub staking_addresses: Vec<StakedStateAddress>,
    pub view_key: PublicKey,
    pub private_key: PrivateKey,
}
pub type WalletInfos = Vec<WalletInfo>;

// constanct connection
// using ws://localhost:26657/websocket
pub struct WebsocketRpc {
    core: Option<MyQueue>,
    websocket_url: String,
}

impl WebsocketRpc {
    pub fn new(websocket_url: String) -> Self {
        Self {
            core: None,
            websocket_url,
        }
    }

    pub fn start_sync<S: Storage + 'static, C: Client + 'static, H: BlockHandler + 'static>(
        &mut self,
        sender: Sender<OwnedMessage>,
        wallet_infos: WalletInfos,
        client: C,
        storage: S,
        handler: H,
    ) -> std::sync::mpsc::Sender<OwnedMessage> {
        let mut core = WebsocketCore::new(sender.clone(), storage, client, handler, wallet_infos);
        self.core = Some(core.get_queue());
        let ret = core.get_queue().clone();
        let _child = thread::spawn(move || {
            core.start();
        });
        ret
    }

    pub fn run<S: Storage + 'static, C: Client + 'static, H: BlockHandler + 'static>(
        &mut self,
        wallets: WalletInfos,
        client: C,
        storage: S,
        block_handler: H,
    ) {
        println!("Connecting to {}", self.websocket_url);
        let mut runtime = tokio::runtime::current_thread::Builder::new()
            .build()
            .unwrap();
        let channel = mpsc::channel(0);
        // tx, rx
        let (channel_tx, channel_rx) = channel;
        // get synchronous sink
        let mut channel_sink = channel_tx.clone().wait();
        self.start_sync(channel_tx.clone(), wallets, client, storage, block_handler);

        let runner = ClientBuilder::new(&self.websocket_url)
            .unwrap()
            .add_protocol("rust-websocket")
            .async_connect_insecure()
            .and_then(|(duplex, _)| {
                channel_sink
                    .send(OwnedMessage::Text(CMD_SUBSCRIBE.to_string()))
                    .unwrap();
                let (sink, stream) = duplex.split();

                stream
                    .filter_map(|message| match message {
                        OwnedMessage::Text(a) => {
                            if let Some(core) = self.core.as_ref() {
                                core.send(OwnedMessage::Text(a.clone())).unwrap();
                            }

                            None
                        }
                        OwnedMessage::Binary(_a) => None,
                        OwnedMessage::Close(e) => Some(OwnedMessage::Close(e)),
                        OwnedMessage::Ping(d) => Some(OwnedMessage::Pong(d)),
                        _ => None,
                    })
                    .select(channel_rx.map_err(|_| WebSocketError::NoDataAvailable))
                    .forward(sink)
            });
        runtime.block_on(runner).unwrap();
    }
}
