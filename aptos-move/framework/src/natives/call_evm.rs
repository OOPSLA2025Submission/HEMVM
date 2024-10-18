use super::code::get_move_string;
use aptos_types::vm_status::StatusCode;
use better_any::{Tid, TidAble};
use cfx_evm::{execution::revert_reason_decode, CrossVMParams, FinalizationResult, TXExecutor};
use ethereum_types::{Address, U256};
use move_binary_format::errors::{PartialVMError, PartialVMResult};
use move_core_types::language_storage::{StructTag, TypeTag};
use move_vm_runtime::native_functions::{NativeContext, NativeFunction};
use move_vm_types::{
    loaded_data::runtime_types::Type,
    natives::function::NativeResult,
    pop_arg,
    values::{Struct, Value},
};
use smallvec::smallvec;
use solidity_abi::ABIDecodable;
use std::{collections::VecDeque, ops::Deref, sync::Arc};

pub mod abort_codes {
    pub const INCORRECT_TYPE_TAG: u64 = 0x1;
    pub const EVM_CALL_REVERT: u64 = 0x2;
    pub const CANNOT_CALL_EVM: u64 = 0x3;
    pub const MALFORMED_OUTPUT: u64 = 0x4;
}
#[derive(Tid)]
pub struct CrossVMContext<'a> {
    pub executor: TXExecutor<'a>,
}

fn make_cross_vm_params(
    struct_tag: Box<StructTag>,
    arguments: &mut VecDeque<Value>,
) -> PartialVMResult<CrossVMParams> {
    let err = PartialVMError::new(StatusCode::UNKNOWN_INVARIANT_VIOLATION_ERROR);

    let gas = U256::from(10_000_000);
    let gas_price = U256::from(1);

    let _call_cap = arguments.pop_back().ok_or_else(|| err.clone())?;

    let evm_params = pop_arg!(arguments, Vec<Value>)
        .into_iter()
        .map(|x| x.value_as::<Vec<u8>>())
        .collect::<PartialVMResult<Vec<Vec<u8>>>>()?;

    let function_name_value = arguments.pop_back().ok_or_else(|| err.clone())?;
    let function_name = get_move_string(function_name_value)?;

    let receiver_raw = pop_arg!(arguments, Vec<u8>);
    let receiver = if receiver_raw.len() == 20 {
        Address::from_slice(&receiver_raw)
    } else {
        return Err(err);
    };

    let mut maybe_coin = pop_arg!(arguments, Struct)
        .unpack()?
        .next()
        .ok_or_else(|| err.clone())?
        .value_as::<Vec<Value>>()?;
    assert!(maybe_coin.len() <= 1);

    let value = if let Some(coin) = maybe_coin.pop() {
        let value = coin
            .value_as::<Struct>()?
            .unpack()?
            .next()
            .ok_or_else(|| err.clone())?
            .value_as::<u64>()?;
        U256::from(value) * U256::from(10_000_000_000u64)
    } else {
        U256::zero()
    };

    let caller_info = {
        let address = struct_tag.address.into_bytes();
        let module = struct_tag.module.as_ident_str().as_str();
        let name = struct_tag.name.as_ident_str().as_str();
        format!("0x{}::{module}::{name}", hex::encode(&address))
    };

    let params = CrossVMParams {
        receiver,
        function_name,
        gas,
        gas_price,
        value,
        evm_params,
        caller_info,
    };
    Ok(params)
}
// cross_vm:: call_evm
fn native_call_evm(
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut arguments: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    let struct_tag = if let TypeTag::Struct(struct_tag) = context.type_to_type_tag(&ty_args[0])? {
        struct_tag
    } else {
        return Ok(NativeResult::err(0.into(), abort_codes::INCORRECT_TYPE_TAG));
    };
    if !struct_tag.type_params.is_empty() {
        return Ok(NativeResult::err(0.into(), abort_codes::INCORRECT_TYPE_TAG));
    }

    let executor = if let Some(CrossVMContext { executor }) =
        context.extensions_mut().get_mut::<Option<CrossVMContext>>()
    {
        executor
    } else {
        println!("Pick Executor Fail");
        println!("Custom backtrace: {}", std::backtrace::Backtrace::force_capture());
        return Ok(NativeResult::err(0.into(), abort_codes::CANNOT_CALL_EVM));
    };

    let params = make_cross_vm_params(struct_tag, &mut arguments)?;
    // println!("Call to EVM Params: {:?}", &params);
    let output = executor.cross_vm_call(params).expect("no db error");

    // FIXME(zeroxg): we drop the substate in this demo
    // substate.accrue(output.substate);
    match output.result {
        Err(err) => {
            println!("EVM Execution Error: {:?}", err);
            Ok(NativeResult::err(0.into(), abort_codes::EVM_CALL_REVERT))
        },
        Ok(FinalizationResult {
            apply_state: false,
            return_data,
            ..
        }) => {
            println!("{:?}", return_data);
            println!(
                "EVM Reverted {:?}",
                revert_reason_decode(return_data.deref())
            );
            Ok(NativeResult::err(0.into(), abort_codes::EVM_CALL_REVERT))
        },
        Ok(res) => {
            let decoded = if res.return_data.is_empty() {
                vec![]
            } else {
                match Vec::<u8>::abi_decode(res.return_data.as_ref()) {
                    Err(_) => {
                        return Ok(NativeResult::err(0.into(), abort_codes::MALFORMED_OUTPUT));
                    },
                    Ok(v) => v,
                }
            };
            match String::from_utf8(decoded.clone()) {
                Ok(string) => {
                    // println!("Decode output success (string): {}", string);
                },
                Err(_) => {
                    // println!("Decode output success (hex): 0x{}", hex::encode(&decoded));
                },
            }

            Ok(NativeResult::ok(0.into(), smallvec![Value::vector_u8(
                decoded
            )]))
        },
    }
}

pub fn make_all() -> impl Iterator<Item = (String, NativeFunction)> {
    let func: NativeFunction =
        Arc::new(move |context, ty_args, args| native_call_evm(context, ty_args, args));

    let natives = [("call_evm", func)];

    crate::natives::helpers::make_module_natives(natives)
}
