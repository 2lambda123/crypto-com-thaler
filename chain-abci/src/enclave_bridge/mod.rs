use enclave_protocol::{IntraEnclaveRequest, IntraEnclaveResponse};

/// TODO: feature-guard when workspaces can be built with --features flag: https://github.com/rust-lang/cargo/issues/5015
pub mod mock;

#[cfg(all(not(feature = "mock-enclave"), target_os = "linux"))]
pub mod real;

/// Abstracts over communication with an external part that does enclave calls
pub trait EnclaveProxy: Sync + Send + Sized {
    // sanity check for checking enclave initialization
    fn check_chain(&self, network_id: u8) -> Result<(), ()>;
    fn process_request(&mut self, request: IntraEnclaveRequest) -> IntraEnclaveResponse;
}
