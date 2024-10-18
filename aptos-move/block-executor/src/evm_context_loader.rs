use crate::{
    aptos_vm::AptosVM, errors::expect_only_successful_execution, logging::AdapterLogSchema, move_vm_ext::{MoveResolverExt, SessionId}, system_module_names::{
        BLOCK_MODULE, GET_BLOCK_HEIGHT_NAME, GET_TIMESTAMP_NAME, TIMESTAMP_MODULE,
    }
};
use aptos_evm::EvmContextReader;
use aptos_types::vm_status::VMStatus;
use move_core_types::{identifier::IdentStr, language_storage::ModuleId, value::MoveValue};
use move_vm_types::gas::UnmeteredGasMeter;

pub struct ContextView<'a, S: MoveResolverExt> {
    aptos_vm: &'a AptosVM,
    storage: &'a S,
    log_context: &'a AdapterLogSchema,
}

impl<'a, S: MoveResolverExt> ContextView<'a, S> {
    pub fn new(aptos_vm: &'a AptosVM, storage: &'a S, log_context: &'a AdapterLogSchema) -> Self {
        Self {
            aptos_vm,
            storage,
            log_context,
        }
    }

    fn view_framework(
        &self,
        module: &'a ModuleId,
        function: &'a IdentStr,
    ) -> Result<MoveValue, VMStatus> {
        let mut gas_meter = UnmeteredGasMeter;
        let mut session = self.aptos_vm.0.new_session(self.storage, SessionId::Void);
        let mut return_values = match session.execute_function_bypass_visibility(
            module,
            function,
            vec![],
            Vec::<Vec<u8>>::new(),
            &mut gas_meter,
        ) {
            Ok(return_vals) => return_vals.return_values,
            Err(e) => expect_only_successful_execution(e, function.as_str(), self.log_context)?,
        };

        assert!(return_values.len() == 1);
        let (raw, ty) = return_values.pop().unwrap();
        let move_value = MoveValue::simple_deserialize(&raw, &ty).unwrap();
        Ok(move_value)
    }
}

fn convert_to_u64(move_value: MoveValue) -> u64 {
    use MoveValue::*;
    match move_value {
        U8(x) => x as u64,
        U64(x) => x as u64,
        U128(x) => x as u64,
        U16(x) => x as u64,
        U32(x) => x as u64,
        U256(x) => x.unchecked_as_u64(),
        Bool(_) | Address(_) | Vector(_) | Struct(_) | Signer(_) => {
            unreachable!()
        },
    }
}

impl<'a, S: MoveResolverExt> EvmContextReader for ContextView<'a, S> {
    fn get_timestamp(&self) -> u64 {
        convert_to_u64(
            self.view_framework(&TIMESTAMP_MODULE, GET_TIMESTAMP_NAME)
                .unwrap(),
        )
    }

    fn get_block_height(&self) -> u64 {
        convert_to_u64(
            self.view_framework(&BLOCK_MODULE, GET_BLOCK_HEIGHT_NAME)
                .unwrap(),
        )
    }
}
