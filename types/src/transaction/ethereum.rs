use cfx_primitives::Action as EthAction;
pub use ethereum_types::{Address as EthAddress, U256};
use serde::{Deserialize, Serialize};

// TODO(lpl): Cannot use `cfx_primitives::Action` because `TransactionPayload` derives `Hash`.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Create creates new contract.
    Create,
    /// Calls contract at given address.
    /// In the case of a transfer, this is the receiver's address.'
    Call(EthAddress),
}

impl Default for Action {
    fn default() -> Action {
        Action::Create
    }
}

impl From<EthAction> for Action {
    fn from(value: EthAction) -> Self {
        match value {
            EthAction::Create => Self::Create,
            EthAction::Call(address) => Self::Call(address),
        }
    }
}

impl Into<EthAction> for &Action {
    fn into(self) -> EthAction {
        match self {
            Action::Create => EthAction::Create,
            Action::Call(address) => EthAction::Call(*address),
        }
    }
}

impl Into<EthAction> for Action {
    fn into(self) -> EthAction {
        (&self).into()
    }
}

impl Action {
    pub fn to_eth_action(&self) -> EthAction {
        match self {
            Action::Create => EthAction::Create,
            Action::Call(address) => EthAction::Call(*address),
        }
    }
}

#[derive(Debug, Default, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthTransactionPayload {
    pub value: U256,
    pub action: Action,
    pub data: Vec<u8>,
}

impl EthTransactionPayload {
    pub fn new(value: U256, action: Action, data: Vec<u8>) -> Self {
        Self {
            value,
            action,
            data,
        }
    }
}
