use crate::context::{ContextReader, EvmContext};
use cfx_evm::{new_machine_with_builtin, CommonParams, Machine, VmFactory};

pub struct EvmMachine {
    pub(crate) inner: Machine,
}

impl EvmMachine {
    pub fn new() -> EvmMachine {
        let params = CommonParams::default();
        let vm_factory = VmFactory::new(1024 * 10);
        let machine = new_machine_with_builtin(params, vm_factory);
        Self { inner: machine }
    }

    pub fn make_context(&self, reader: &impl ContextReader) -> EvmContext {
        EvmContext::make_for_new_block(reader, &self.inner)
    }
}
