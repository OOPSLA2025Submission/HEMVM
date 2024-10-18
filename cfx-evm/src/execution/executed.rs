// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use crate::{bytes::Bytes, vm};
use cfx_types::{AddressWithSpace, U256, U512};
use primitives::LogEntry;
use solidity_abi::{ABIDecodable, ABIDecodeError};

#[derive(Debug, PartialEq, Clone)]
pub struct Executed {
    /// Gas used during execution of transaction.
    pub gas_used: U256,

    /// Fee that need to be paid by execution of this transaction.
    pub fee: U256,

    /// Gas charged during execution of transaction.
    pub gas_charged: U256,

    /// Vector of logs generated by transaction.
    pub logs: Vec<LogEntry>,

    /// Addresses of contracts created during execution of transaction.
    /// Ordered from earliest creation.
    ///
    /// eg. sender creates contract A and A in constructor creates contract B
    ///
    /// B creation ends first, and it will be the first element of the vector.
    pub contracts_created: Vec<AddressWithSpace>,
    /// Transaction output.
    pub output: Bytes,
    /// The trace of this transaction.
    pub trace: Vec<ExecTrace>,
    /// Only for the virtual call, an accurate gas estimation for gas usage,
    pub estimated_gas_limit: Option<U256>,
}

#[derive(Debug)]
pub enum ToRepackError {
    /// Returned when transaction nonce does not match state nonce.
    InvalidNonce {
        /// Nonce expected.
        expected: U256,
        /// Nonce found.
        got: U256,
    },

    /// Returned when a non-sponsored transaction's sender does not exist yet.
    SenderDoesNotExist,
}

#[derive(Debug)]
pub enum TxDropError {
    /// The account nonce in world-state is larger than tx nonce
    OldNonce(U256, U256),
    ///
    NotEnoughBaseGas { expected: u64, actual: u64 },
}

#[derive(Debug, PartialEq)]
pub enum ExecutionError {
    /// Returned when cost of transaction (value + gas_price * gas) exceeds
    /// current sender balance.
    NotEnoughCash {
        /// Minimum required balance.
        required: U512,
        /// Actual balance.
        got: U512,
        /// Actual gas cost. This should be min(gas_fee, balance).
        actual_gas_cost: U256,
    },
    VmError(vm::Error),
}

#[derive(Debug)]
pub enum ExecutionOutcome {
    NotExecutedDrop(TxDropError),
    NotExecutedToReconsiderPacking(ToRepackError),
    ExecutionErrorBumpNonce(ExecutionError, Executed),
    Finished(Executed),
}

impl ExecutionOutcome {
    pub fn successfully_executed(self) -> Option<Executed> {
        match self {
            ExecutionOutcome::Finished(executed) => Some(executed),
            _ => None,
        }
    }
}

impl Executed {
    pub fn not_enough_balance_fee_charged(
        tx: &impl TransactionInfo,
        fee: &U256,
        trace: Vec<ExecTrace>,
        _spec: &Spec,
    ) -> Self {
        let gas_charged = if *tx.gas_price() == U256::zero() {
            U256::zero()
        } else {
            fee / *tx.gas_price()
        };
        Self {
            gas_used: *tx.gas(),
            gas_charged,
            fee: fee.clone(),
            logs: vec![],
            contracts_created: vec![],
            output: Default::default(),
            trace,
            estimated_gas_limit: None,
        }
    }

    pub fn execution_error_fully_charged(
        tx: &impl TransactionInfo,
        trace: Vec<ExecTrace>,
        _spec: &Spec,
    ) -> Self {
        Self {
            gas_used: *tx.gas(),
            gas_charged: *tx.gas(),
            fee: tx.gas().saturating_mul(*tx.gas_price()),
            logs: vec![],
            contracts_created: vec![],

            output: Default::default(),
            trace,
            estimated_gas_limit: None,
        }
    }
}

pub fn revert_reason_decode(output: &[u8]) -> String {
    const MAX_LENGTH: usize = 50;
    let decode_result = if output.len() < 4 {
        Err(ABIDecodeError("Uncompleted Signature"))
    } else {
        let (sig, data) = output.split_at(4);
        if sig != [8, 195, 121, 160] {
            Err(ABIDecodeError("Unrecognized Signature"))
        } else {
            String::abi_decode(data)
        }
    };
    match decode_result {
        Ok(str) => {
            if str.len() < MAX_LENGTH {
                str
            } else {
                format!("{}...", str[..MAX_LENGTH].to_string())
            }
        },
        Err(_) => format!("0x{}", hex::encode(output)),
    }
}

use super::transaction_info::TransactionInfo;
use crate::{observer::trace::ExecTrace, vm::Spec};
#[cfg(test)]
use rustc_hex::FromHex;

#[test]
fn test_decode_result() {
    let input_hex = "08c379a0\
                     0000000000000000000000000000000000000000000000000000000000000020\
                     0000000000000000000000000000000000000000000000000000000000000018\
                     e699bae59586e4b88de8b6b3efbc8ce8afb7e58585e580bc0000000000000000";
    assert_eq!(
        "智商不足，请充值".to_string(),
        revert_reason_decode(&input_hex.from_hex().unwrap())
    );
}