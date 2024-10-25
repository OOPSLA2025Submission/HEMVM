use super::{
    executed::{Executed, ExecutionError, ExecutionOutcome, ToRepackError, TxDropError},
    transaction_info::TransactionInfo,
    TransactOptions,
};
use crate::{
    call_create_frame::{contract_address, CallCreateFrame, FrameStack, FrameStackOutput},
    evm::FinalizationResult,
    machine::Machine,
    observer::{AddressPocket, MultiObservers, StateTracer},
    state::{cleanup_mode, Substate},
    vm::{self, ActionParams, ActionValue, CallType, CreateContractAddress, CreateType, Env, Spec},
    vm_factory::VmFactory,
};
use cfx_parameters::internal_contract_addresses::CROSS_SPACE_CONTRACT_ADDRESS;
use cfx_state::StateTrait;
use cfx_statedb::Result as DbResult;
use cfx_types::{Address, AddressSpaceUtil, AddressWithSpace, Space, U256, U512};
use primitives::transaction::Action;
use solidity_abi::ABIEncodable;
use std::{
    collections::HashSet,
    convert::{TryFrom, TryInto},
    sync::Arc,
};

/// Transaction executor.
pub struct TXExecutor<'a> {
    pub(super) state: &'a mut dyn StateTrait,
    env: &'a Env,
    machine: &'a Machine,
    factory: VmFactory,
    pub(super) spec: &'a Spec,
}

pub fn gas_required_for(is_create: bool, data: &[u8], spec: &Spec) -> u64 {
    data.iter().fold(
        (if is_create {
            spec.tx_create_gas
        } else {
            spec.tx_gas
        }) as u64,
        |g, b| {
            g + (match *b {
                0 => spec.tx_data_zero_gas,
                _ => spec.tx_data_non_zero_gas,
            }) as u64
        },
    )
}

enum PreCheckResult<'a> {
    Pass {
        top_frame: CallCreateFrame<'a>,
        frame_stack: FrameStack<'a>,
    },
    Fail(ExecutionOutcome),
}

#[derive(Debug)]
pub struct CrossVMParams {
    pub receiver: Address,
    pub function_name: String,
    pub gas: U256,
    pub gas_price: U256,
    pub value: U256,
    pub evm_params: Vec<Vec<u8>>,
    pub caller_info: String,
}

pub struct CrossVMReturn {
    pub substate: Substate,
    pub result: vm::Result<FinalizationResult>,
}

impl CrossVMParams {
    fn data(&self) -> Vec<u8> {
        if self.function_name.is_empty() {
            return vec![];
        }
        let func_sig = format!("{}(string,bytes[])", self.function_name);
        let func_bytes = keccak_hash::keccak(func_sig);
        let abi_encode = (self.caller_info.clone(), self.evm_params.clone()).abi_encode();
        [&func_bytes[..4], &abi_encode[..]].concat()
    }
}

impl<'a> TXExecutor<'a> {
    /// Basic constructor.
    pub fn new(
        state: &'a mut dyn StateTrait,
        env: &'a Env,
        machine: &'a Machine,
        spec: &'a Spec,
    ) -> Self {
        TXExecutor {
            state,
            env,
            machine,
            factory: machine.vm_factory(),
            spec,
        }
    }

    pub fn transact(
        &mut self,
        tx: &impl TransactionInfo,
        options: TransactOptions,
    ) -> DbResult<ExecutionOutcome> {
        let pre_check_result = self.transact_preprocessing(tx, options)?;

        let (top_frame, frame_stack) = match pre_check_result {
            PreCheckResult::Pass {
                top_frame,
                frame_stack,
            } => (top_frame, frame_stack),
            PreCheckResult::Fail(outcome) => {
                return Ok(outcome);
            },
        };

        let frame_stack_output = frame_stack.exec(top_frame)?; // stopped here

        Ok(self.transact_postprocessing(tx, frame_stack_output)?)
    }

    pub fn cross_vm_call(&mut self, params: CrossVMParams) -> DbResult<CrossVMReturn> {
        let pre_check_result = self.cross_vm_call_preprocessing(params)?;

        let (top_frame, frame_stack) = match pre_check_result {
            PreCheckResult::Pass {
                top_frame,
                frame_stack,
            } => (top_frame, frame_stack),
            PreCheckResult::Fail(_) => unreachable!(),
        };

        let FrameStackOutput {
            substate, result, ..
        } = frame_stack.exec(top_frame)?;
        return Ok(CrossVMReturn { substate, result });
    }

    fn transact_preprocessing(
        &mut self,
        tx: &impl TransactionInfo,
        options: TransactOptions,
    ) -> DbResult<PreCheckResult> {
        let TransactOptions {
            mut observer,
            check_settings,
        } = options;

        let spec = self.spec;
        let sender = tx.sender();
        let nonce = self.state.nonce(&sender)?;

        // Validate transaction nonce
        if *tx.nonce() < nonce {
            return Ok(PreCheckResult::Fail(ExecutionOutcome::NotExecutedDrop(
                TxDropError::OldNonce(nonce, *tx.nonce()),
            )));
        } else if *tx.nonce() > nonce {
            return Ok(PreCheckResult::Fail(
                ExecutionOutcome::NotExecutedToReconsiderPacking(ToRepackError::InvalidNonce {
                    expected: nonce,
                    got: *tx.nonce(),
                }),
            ));
        }

        let base_gas_required =
            gas_required_for(&*tx.action() == &Action::Create, &tx.data(), spec);
        if *tx.gas() < base_gas_required.into() {
            return Ok(PreCheckResult::Fail(ExecutionOutcome::NotExecutedDrop(
                TxDropError::NotEnoughBaseGas {
                    expected: base_gas_required,
                    actual: tx.gas().as_u64(),
                },
            )));
        }

        let balance = self.state.balance(&sender)?;
        let gas_cost = if check_settings.charge_gas {
            tx.gas().full_mul(*tx.gas_price())
        } else {
            0.into()
        };

        let sender_balance = U512::from(balance);

        let total_cost = U512::from(*tx.value()) + gas_cost;

        let mut tx_substate = Substate::new();
        if sender_balance < total_cost {
            // Sender is responsible for the insufficient balance.
            // Sub tx fee if not enough cash, and substitute all remaining
            // balance if balance is not enough to pay the tx fee
            let actual_gas_cost: U256 = U512::min(gas_cost, sender_balance).try_into().unwrap();

            // We don't want to bump nonce for non-existent account when we
            // can't charge gas fee. In this case, the sender account will
            // not be created if it does not exist.
            if !self.state.exists(&sender)? && check_settings.real_execution {
                return Ok(PreCheckResult::Fail(
                    ExecutionOutcome::NotExecutedToReconsiderPacking(
                        ToRepackError::SenderDoesNotExist,
                    ),
                ));
            }
            self.state
                .inc_nonce(&sender, &self.spec.account_start_nonce)?;
            self.state.sub_balance(
                &sender,
                &actual_gas_cost,
                &mut cleanup_mode(&mut tx_substate, &spec),
            )?;
            observer.as_state_tracer().trace_internal_transfer(
                AddressPocket::Balance(sender.address.with_space(tx.space())),
                AddressPocket::GasPayment,
                actual_gas_cost,
            );

            return Ok(PreCheckResult::Fail(
                ExecutionOutcome::ExecutionErrorBumpNonce(
                    ExecutionError::NotEnoughCash {
                        required: total_cost,
                        got: sender_balance,
                        actual_gas_cost: actual_gas_cost.clone(),
                    },
                    Executed::not_enough_balance_fee_charged(
                        tx,
                        &actual_gas_cost,
                        observer.tracer.map_or(Default::default(), |t| t.drain()),
                        &self.spec,
                    ),
                ),
            ));
        } else {
            // From now on sender balance >= total_cost, even if the sender
            // account does not exist (since she may be sponsored). Transaction
            // execution is guaranteed. Note that inc_nonce() will create a
            // new account if the account does not exist.
            self.state
                .inc_nonce(&sender, &self.spec.account_start_nonce)?;
        }

        // Subtract the transaction fee from sender or contract.
        let gas_cost = U256::try_from(gas_cost).unwrap();

        {
            observer.as_state_tracer().trace_internal_transfer(
                AddressPocket::Balance(sender.address.with_space(tx.space())),
                AddressPocket::GasPayment,
                gas_cost,
            );
            self.state.sub_balance(
                &sender,
                &U256::try_from(gas_cost).unwrap(),
                &mut cleanup_mode(&mut tx_substate, &spec),
            )?;
        }

        let init_gas = *tx.gas() - base_gas_required;

        let top_frame = match *tx.action() {
            Action::Create => {
                let address_scheme = match tx.space() {
                    Space::Ethereum => CreateContractAddress::FromSenderNonce,
                };
                let (new_address, _code_hash) = contract_address(
                    address_scheme,
                    self.env.number.into(),
                    &sender,
                    &nonce,
                    &tx.data(),
                );

                let params = ActionParams {
                    space: sender.space,
                    code_address: new_address.address,
                    code_hash: None,
                    address: new_address.address,
                    sender: sender.address,
                    original_sender: sender.address,
                    gas: init_gas,
                    gas_price: *tx.gas_price(),
                    value: ActionValue::Transfer(*tx.value()),
                    code: Some(Arc::new(tx.data().into_owned())),
                    data: None,
                    call_type: CallType::None,
                    create_type: CreateType::CREATE,
                    params_type: vm::ParamsType::Embedded,
                };
                CallCreateFrame::new_create_raw(
                    params,
                    self.env,
                    self.machine,
                    self.spec,
                    &self.factory,
                    0,     /* depth */
                    false, /* static_flag */
                )
            },
            Action::Call(ref address) => {
                let address = address.with_space(sender.space);
                let params = ActionParams {
                    space: sender.space,
                    code_address: address.address,
                    address: address.address,
                    sender: sender.address,
                    original_sender: sender.address,
                    gas: init_gas,
                    gas_price: *tx.gas_price(),
                    value: ActionValue::Transfer(*tx.value()),
                    code: self.state.code(&address)?,
                    code_hash: self.state.code_hash(&address)?,
                    data: Some(tx.data().into_owned()),
                    call_type: CallType::Call,
                    create_type: CreateType::None,
                    params_type: vm::ParamsType::Separate,
                };
                CallCreateFrame::new_call_raw(
                    params,
                    self.env,
                    self.machine,
                    self.spec,
                    &self.factory,
                    0,     /* depth */
                    false, /* static_flag */
                )
            },
        };

        let frame_stack = FrameStack::new(self.state, tx_substate, observer, base_gas_required);

        Ok(PreCheckResult::Pass {
            top_frame,
            frame_stack,
        })
    }

    /// Finalizes the transaction (does refunds and suicides).
    fn transact_postprocessing(
        &mut self,
        tx: &impl TransactionInfo,
        frame_stack_output: FrameStackOutput,
    ) -> DbResult<ExecutionOutcome> {
        let FrameStackOutput {
            mut substate,
            result,
            mut observer,
            base_gas_required,
        } = frame_stack_output;

        let output = result
            .as_ref()
            .map(|res| res.return_data.to_vec())
            .unwrap_or_default();

        let estimated_gas_limit = observer
            .gas_man
            .as_ref()
            .map(|g| g.gas_required() * 7 / 6 + base_gas_required);

        let gas_left = match result {
            Ok(FinalizationResult { gas_left, .. }) => gas_left,
            _ => 0.into(),
        };

        // gas_used is only used to estimate gas needed
        let gas_used = *tx.gas() - gas_left;
        // gas_left should be smaller than 1/4 of gas_limit, otherwise
        // 3/4 of gas_limit is charged.
        let charge_all = (gas_left + gas_left + gas_left) >= gas_used;
        let (gas_charged, fees_value, refund_value) = if charge_all {
            let gas_refunded = *tx.gas() >> 2;
            let gas_charged = *tx.gas() - gas_refunded;
            (
                gas_charged,
                gas_charged.saturating_mul(*tx.gas_price()),
                gas_refunded.saturating_mul(*tx.gas_price()),
            )
        } else {
            (
                gas_used,
                gas_used.saturating_mul(*tx.gas_price()),
                gas_left.saturating_mul(*tx.gas_price()),
            )
        };

        {
            observer.as_state_tracer().trace_internal_transfer(
                AddressPocket::GasPayment,
                AddressPocket::Balance(tx.sender().into_owned()),
                refund_value.clone(),
            );
            self.state.add_balance(
                &tx.sender(),
                &refund_value,
                cleanup_mode(&mut substate, self.spec),
                self.spec.account_start_nonce,
            )?;
        };

        // perform suicides

        let subsubstate = self.kill_process(&substate.suicides, observer.as_state_tracer())?;
        substate.accrue(subsubstate);

        // TODO should be added back after enabling dust collection
        // Should be executed once per block, instead of per transaction?
        //
        // When enabling this feature, remember to check touched set in
        // functions like "add_collateral_for_storage()" in "State"
        // struct.

        //        // perform garbage-collection
        //        let min_balance = if spec.kill_dust != CleanDustMode::Off {
        //            Some(U256::from(spec.tx_gas) * tx.gas_price())
        //        } else {
        //            None
        //        };
        //
        //        self.state.kill_garbage(
        //            &substate.touched,
        //            spec.kill_empty,
        //            &min_balance,
        //            spec.kill_dust == CleanDustMode::WithCodeAndStorage,
        //        )?;

        match result {
            Err(vm::Error::StateDbError(e)) => bail!(e.0),
            Err(exception) => Ok(ExecutionOutcome::ExecutionErrorBumpNonce(
                ExecutionError::VmError(exception),
                Executed::execution_error_fully_charged(
                    tx,
                    observer.tracer.map_or(Default::default(), |t| t.drain()),
                    &self.spec,
                ),
            )),
            Ok(r) => {
                let trace = observer.tracer.map_or(Default::default(), |t| t.drain());

                let executed = Executed {
                    gas_used,
                    gas_charged,
                    fee: fees_value,
                    logs: substate.logs.to_vec(),
                    contracts_created: substate.contracts_created.to_vec(),
                    output,
                    trace,
                    estimated_gas_limit,
                };

                if r.apply_state {
                    Ok(ExecutionOutcome::Finished(executed))
                } else {
                    // Transaction reverted by vm instruction.
                    Ok(ExecutionOutcome::ExecutionErrorBumpNonce(
                        ExecutionError::VmError(vm::Error::Reverted),
                        executed,
                    ))
                }
            },
        }
    }

    fn cross_vm_call_preprocessing(
        &mut self,
        cross_vm_params: CrossVMParams,
    ) -> DbResult<PreCheckResult> {
        let data = cross_vm_params.data();
        let CrossVMParams {
            receiver,
            gas,
            gas_price,
            value,
            ..
        } = cross_vm_params;
        let address = receiver.with_evm_space();

        let params = ActionParams {
            space: address.space,
            code_address: address.address,
            address: address.address,
            sender: *CROSS_SPACE_CONTRACT_ADDRESS,
            original_sender: Address::zero(),
            gas,
            gas_price,
            value: ActionValue::Transfer(value),
            code: self.state.code(&address)?,
            code_hash: self.state.code_hash(&address)?,
            data: Some(data),
            call_type: CallType::Call,
            create_type: CreateType::None,
            params_type: vm::ParamsType::Separate,
        };
        let top_frame = CallCreateFrame::new_call_raw(
            params,
            self.env,
            self.machine,
            self.spec,
            &self.factory,
            0,     /* depth */
            false, /* static_flag */
        );
        let mut substate = Substate::new();
        self.state.add_balance(
            &CROSS_SPACE_CONTRACT_ADDRESS.with_evm_space(),
            &value,
            cleanup_mode(&mut substate, &self.spec),
            self.spec.account_start_nonce,
        )?;

        let frame_stack = FrameStack::new(
            self.state,
            Substate::new(),
            MultiObservers::with_no_tracing(),
            0,
        );

        Ok(PreCheckResult::Pass {
            top_frame,
            frame_stack,
        })
    }

    // TODO: maybe we can find a better interface for doing the suicide
    // post-processing.
    fn kill_process(
        &mut self,
        suicides: &HashSet<AddressWithSpace>,
        tracer: &mut dyn StateTracer,
    ) -> DbResult<Substate> {
        let substate = Substate::new();

        for contract_address in suicides {
            let contract_balance = self.state.balance(contract_address)?;
            tracer.trace_internal_transfer(
                AddressPocket::Balance(*contract_address),
                AddressPocket::MintBurn,
                contract_balance.clone(),
            );

            self.state.remove_contract(contract_address)?;
            self.state.subtract_total_issued(contract_balance);
        }

        Ok(substate)
    }
}
