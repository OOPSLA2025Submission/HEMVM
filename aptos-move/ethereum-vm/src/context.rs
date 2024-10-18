use cfx_evm::{Env, Machine, Spec};
use cfx_types::{Address, H256, U256};

pub trait ContextReader {
    fn get_timestamp(&self) -> u64;
    fn get_block_height(&self) -> u64;
}

pub struct EvmContext {
    pub(crate) env: Env,
    pub(crate) spec: Spec,
}

// Question: stubs?
impl EvmContext {
    pub(crate) fn make_for_new_block(reader: &impl ContextReader, machine: &Machine) -> Self {
        // FIXME(vm): panics here.
        let block_height = reader.get_block_height();
        let timestamp = reader.get_timestamp();
        let env = Env {
            number: block_height,
            author: Address::zero(),
            timestamp,
            difficulty: U256::zero(),
            gas_limit: U256::zero(),
            last_hash: H256::zero(),
            accumulated_gas_used: U256::zero(),
            epoch_height: block_height,
        };
        let spec = machine.params().spec(block_height);
        EvmContext { env, spec }
    }
}
