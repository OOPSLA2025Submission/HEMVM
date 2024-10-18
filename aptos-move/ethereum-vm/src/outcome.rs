use aptos_types::{
    transaction::{AbortInfo, ExecutionStatus, TransactionStatus},
    vm_status::StatusCode,
};
use aptos_logger::prelude::*;
use cfx_evm::{
    execution::{revert_reason_decode, Executed, ExecutionError, ToRepackError, TxDropError},
    vm::Error as VmError,
    ExecutionOutcome,
};
use move_core_types::vm_status::{AbortLocation::Script, StatusCode::*, VMStatus};

pub mod abort_code {
    pub const BUILTIN_CODE: u64 = 1029;
    pub const INTERNAL_CONTRACT_CODE: u64 = 1030;
    pub const REVERT_CODE: u64 = 1031;

    pub const BUILTIN_DESC: &'static str = "Built-in function fail";
    pub const INTERNAL_CONTRACT_DESC: &'static str = "Internal contract fail";
    pub const REVERT_DESC: &'static str = "EVM reverted";
}

use abort_code::*;

pub fn convert_exeuction_outcome(outcome: &ExecutionOutcome) -> (VMStatus, TransactionStatus) {
    let vm_status: ConvertedOutcome = outcome.into();
    let mut tx_output: TransactionStatus = vm_status.status.clone().into();
    if let TransactionStatus::Keep(ExecutionStatus::MoveAbort { info, .. }) = &mut tx_output {
        println!("{:#?}", vm_status);
        *info = vm_status.reason;
        match info {
            Some(AbortInfo {
                reason_name,
                description,
            }) if reason_name == REVERT_DESC => {
                *description = extract_error_reason(outcome).unwrap();
                println!("{:#?}", description);
            },
            _ => {},
        }
    }
    return (vm_status.status, tx_output);
}

pub fn extract_evm_executed(outcome: &ExecutionOutcome) -> Option<&Executed> {
    match outcome {
        ExecutionOutcome::NotExecutedDrop(_)
        | ExecutionOutcome::NotExecutedToReconsiderPacking(_) => None,
        ExecutionOutcome::ExecutionErrorBumpNonce(_, executed)
        | ExecutionOutcome::Finished(executed) => Some(executed),
    }
}

fn extract_error_reason(outcome: &ExecutionOutcome) -> Option<String> {
    if let ExecutionOutcome::ExecutionErrorBumpNonce(
        ExecutionError::VmError(VmError::Reverted),
        executed,
    ) = outcome
    {
        Some(revert_reason_decode(&executed.output))
    } else {
        None
    }
}

#[derive(Debug)]
struct ConvertedOutcome {
    status: VMStatus,
    reason: Option<AbortInfo>,
}

impl Into<ConvertedOutcome> for VMStatus {
    fn into(self) -> ConvertedOutcome {
        ConvertedOutcome {
            status: self,
            reason: Default::default(),
        }
    }
}

impl Into<ConvertedOutcome> for StatusCode {
    fn into(self) -> ConvertedOutcome {
        VMStatus::Error(self).into()
    }
}

impl Into<ConvertedOutcome> for &ExecutionOutcome {
    fn into(self) -> ConvertedOutcome {
        match self {
            ExecutionOutcome::NotExecutedDrop(err) => err.into(),
            ExecutionOutcome::NotExecutedToReconsiderPacking(err) => err.into(),
            ExecutionOutcome::ExecutionErrorBumpNonce(err, _) => err.into(),
            ExecutionOutcome::Finished(_) => VMStatus::Executed.into(),
        }
    }
}

impl Into<ConvertedOutcome> for &TxDropError {
    fn into(self) -> ConvertedOutcome {
        match self {
            TxDropError::OldNonce(_, _) => SEQUENCE_NUMBER_TOO_OLD.into(),
            TxDropError::NotEnoughBaseGas { .. } => {
                MAX_GAS_UNITS_BELOW_MIN_TRANSACTION_GAS_UNITS.into()
            },
        }
    }
}

impl Into<ConvertedOutcome> for &ToRepackError {
    fn into(self) -> ConvertedOutcome {
        match self {
            ToRepackError::InvalidNonce { .. } => SEQUENCE_NUMBER_TOO_NEW.into(),
            ToRepackError::SenderDoesNotExist => {
                debug!("SENDING ACCOUNT EVM TRANSFORMATION");
                SENDING_ACCOUNT_DOES_NOT_EXIST.into()
            }
        }
    }
}

impl Into<ConvertedOutcome> for &ExecutionError {
    fn into(self) -> ConvertedOutcome {
        match self {
            ExecutionError::NotEnoughCash { .. } => ARITHMETIC_ERROR.into(),
            ExecutionError::VmError(err) => err.into(),
        }
    }
}

impl Into<ConvertedOutcome> for &VmError {
    fn into(self) -> ConvertedOutcome {
        // All the error code here should be 1xxx, 3xxx or 4xxx
        match self {
            VmError::OutOfGas => OUT_OF_GAS.into(),
            VmError::BadJumpDestination { .. } => FUNCTION_RESOLUTION_FAILURE.into(),
            VmError::BadInstruction { .. } | VmError::InvalidSubEntry => UNKNOWN_OPCODE.into(),

            VmError::StackUnderflow { .. }
            | VmError::OutOfStack { .. }
            | VmError::SubStackUnderflow { .. }
            | VmError::OutOfSubStack { .. } => EXECUTION_STACK_OVERFLOW.into(),

            VmError::NotEnoughBalanceForStorage { .. }
            | VmError::ExceedStorageLimit
            | VmError::Wasm(_)
            | VmError::InvalidAddress(_)
            | VmError::ConflictAddress(_) => unreachable!("Inactivate feature"),

            VmError::BuiltIn(msg) => ConvertedOutcome {
                status: VMStatus::MoveAbort(Script, BUILTIN_CODE),
                reason: Some(AbortInfo {
                    reason_name: BUILTIN_DESC.into(),
                    description: (*msg).into(),
                }),
            },
            VmError::InternalContract(msg) => ConvertedOutcome {
                status: VMStatus::MoveAbort(Script, INTERNAL_CONTRACT_CODE),
                reason: Some(AbortInfo {
                    reason_name: INTERNAL_CONTRACT_DESC.into(),
                    description: msg.into(),
                }),
            },
            VmError::Reverted => ConvertedOutcome {
                status: VMStatus::MoveAbort(Script, REVERT_CODE),
                reason: Some(AbortInfo {
                    reason_name: REVERT_DESC.into(),
                    description: String::new(),
                }),
            },

            VmError::MutableCallInStaticContext => UNKNOWN_RUNTIME_STATUS.into(),
            VmError::StateDbError(_) => UNKNOWN_RUNTIME_STATUS.into(),

            VmError::OutOfBounds => VECTOR_OPERATION_ERROR.into(),
        }
    }
}
