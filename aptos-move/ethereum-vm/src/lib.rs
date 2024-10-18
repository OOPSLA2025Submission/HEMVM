mod context;
mod events;
mod machine;
mod outcome;
mod state;
mod storage_key;
mod transaction;

pub use cfx_evm::{
    vm::Error as EvmError, ExecutionOutcome, TXExecutor, TransactOptions, TransactionInfo,
};
pub use cfx_primitives::Action;
pub use cfx_types::{Address, AddressWithSpace, Space, U256};
pub use context::{ContextReader as EvmContextReader, EvmContext};
pub use events::{aptos_events_to_evm_events, evm_events_to_aptos_events};
pub use machine::EvmMachine;
pub use outcome::{convert_exeuction_outcome, extract_evm_executed};
pub use state::{EvmState, ViewWrapper};
pub use transaction::EvmTransaction;

pub fn make_executor<'a>(
    machine: &'a EvmMachine,
    context: &'a EvmContext,
    state: &'a mut EvmState,
) -> TXExecutor<'a> {
    TXExecutor::new(
        &mut state.state,
        &context.env,
        &machine.inner,
        &context.spec,
    )
}
