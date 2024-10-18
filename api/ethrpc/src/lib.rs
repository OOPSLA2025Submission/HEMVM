use crate::{impls::eth::EthHandler, traits::eth::EthServer};
use aptos_api::Context;
use jsonrpsee::{core::Error, server::ServerBuilder};
use tokio::{self, task::JoinHandle};

mod impls;
mod traits;
mod types;
// mod executor;

pub fn bootstrap(
    context: Context,
    runtime_handle: &tokio::runtime::Handle,
) -> Result<JoinHandle<()>, Error> {
    let _guard = runtime_handle.enter();
    let server = runtime_handle
        .block_on(ServerBuilder::default().build(context.node_config.eth_api.address))?;
    let server_handle = server.start(EthHandler::new(context).into_rpc())?;
    let join_handle = runtime_handle.spawn(server_handle.stopped());
    Ok(join_handle)
}
