use abci_enclave_protocol::{read_bincode, send_bincode, SubAbciRequest, SubAbciResponse};
use std::io::{self, Write};
use std::net::TcpStream;
use abci_enclave_macro::get_network_id;

const NETWORK_ID: u8 = get_network_id!();

fn respond(request: SubAbciRequest, stream: &mut Write) -> io::Result<()> {
    let resp = match request {
        SubAbciRequest::InitChain(chain_hex_id) if chain_hex_id == NETWORK_ID => {
            SubAbciResponse::InitChain(true)
        }
        SubAbciRequest::InitChain(_) => SubAbciResponse::InitChain(false),
    };
    send_bincode(&resp, stream)
}

pub fn handle_stream(stream: &mut TcpStream) {
    loop {
        if let Some(request) = read_bincode::<SubAbciRequest>(stream) {
            respond(request, stream).expect("failed to write response");
        } else {
            break;
        };
    }
}
