// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

use anyhow::{format_err, Result};
use aptos_evm::{EvmState, ViewWrapper};
use aptos_state_view::account_with_state_view::AsAccountWithStateView;
use aptos_types::{
    account_address::AccountAddress,
    account_config::AccountSequenceInfo,
    account_view::AccountView,
    on_chain_config::OnChainConfigPayload,
    transaction::{EthAddress, SignedTransaction, VMValidatorResult},
};
use aptos_block_executor::{data_cache::AsMoveResolver, aptos_vm::AptosVM, state_view::{DbReader, DbStateView, LatestDbStateCheckpointView}, cached_state_view::CachedDbStateView};
use cfx_state::state_trait::StateOpsTrait;
use cfx_types::AddressSpaceUtil;
use fail::fail_point;
use std::sync::Arc;

#[cfg(test)]
#[path = "unit_tests/vm_validator_test.rs"]
mod vm_validator_test;

pub trait TransactionValidation: Send + Sync + Clone {
    type ValidationInstance: aptos_block_executor::VMValidator;

    /// Validate a txn from client
    fn validate_transaction(&self, _txn: SignedTransaction) -> Result<VMValidatorResult>;

    /// Restart the transaction validation instance
    fn restart(&mut self, config: OnChainConfigPayload) -> Result<()>;

    /// Notify about new commit
    fn notify_commit(&mut self);
}

pub struct VMValidator {
    db_reader: Arc<dyn DbReader>,
    state_view: CachedDbStateView,
    vm: AptosVM,
}

impl Clone for VMValidator {
    fn clone(&self) -> Self {
        Self::new(self.db_reader.clone())
    }
}

impl VMValidator {
    pub fn new(db_reader: Arc<dyn DbReader>) -> Self {
        let db_state_view = db_reader
            .latest_state_checkpoint_view()
            .expect("Get db view cannot fail");

        let vm = AptosVM::new_for_validation(&db_state_view);
        VMValidator {
            db_reader,
            state_view: db_state_view.into(),
            vm,
        }
    }
}

impl TransactionValidation for VMValidator {
    type ValidationInstance = AptosVM;

    fn validate_transaction(&self, txn: SignedTransaction) -> Result<VMValidatorResult> {
        fail_point!("vm_validator::validate_transaction", |_| {
            Err(anyhow::anyhow!(
                "Injected error in vm_validator::validate_transaction"
            ))
        });
        use aptos_block_executor::VMValidator;

        Ok(self.vm.validate_transaction(txn, &self.state_view))
    }

    fn restart(&mut self, _config: OnChainConfigPayload) -> Result<()> {
        self.notify_commit();

        self.vm = AptosVM::new_for_validation(&self.state_view);
        Ok(())
    }

    fn notify_commit(&mut self) {
        self.state_view = self
            .db_reader
            .latest_state_checkpoint_view()
            .expect("Get db view cannot fail")
            .into();
    }
}

/// returns account's sequence number from storage
pub fn get_account_sequence_number(
    state_view: &DbStateView,
    address: AccountAddress,
) -> Result<AccountSequenceInfo> {
    fail_point!("vm_validator::get_account_sequence_number", |_| {
        Err(anyhow::anyhow!(
            "Injected error in get_account_sequence_number"
        ))
    });
    if let Some(address) = get_eth_address(&address) {
        let mut view_wrapper = ViewWrapper {
            inner: &state_view.as_move_resolver(),
            cache: Default::default(),
        };
        let state = EvmState::new(&mut view_wrapper);
        let nonce = state
            .state
            .nonce(&address.with_evm_space())
            .map_err(|e| format_err!("{:?}", e))?;
        Ok(AccountSequenceInfo::Sequential(nonce.as_u64()))
    } else {
        let account_state_view = state_view.as_account_with_state_view(&address);

        match account_state_view.get_account_resource()? {
            Some(account_resource) => Ok(AccountSequenceInfo::Sequential(
                account_resource.sequence_number(),
            )),
            None => Ok(AccountSequenceInfo::Sequential(0)),
        }
    }
}

fn get_eth_address(address: &AccountAddress) -> Option<EthAddress> {
    if address[EthAddress::len_bytes()..] == [0; 12] {
        Some(EthAddress::from_slice(
            address[..EthAddress::len_bytes()].into(),
        ))
    } else {
        None
    }
}
