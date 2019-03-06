#![feature(proc_macro_hygiene)]

use std::io;
use std::net::TcpListener;
use std::env;
mod server;

fn main() -> io::Result<()> {
    // TODO: custom runner with args, TLS, remote attestation...
    let address = env::args().next().unwrap_or("127.0.0.1:7878".to_string());
    for stream in TcpListener::bind(address)?.incoming() {
        let mut stream = stream?;
        server::handle_stream(&mut stream);
    }
    Ok(())
}
