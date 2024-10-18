use crate::{
    traits::eth::EthServer,
    types::{
        block::{Block, BlockTransactions},
        block_number::BlockNumber,
        call_request::{sign_call, CallRequest},
        log::Log,
        receipts::Receipt,
        transaction::{deployed_contract_address, Transaction as RpcTransaction},
    },
};

use aptos_block_executor::state_view::DbStateViewAtVersion;
use aptos_api::Context;
use aptos_api_types::HexEncodedBytes;
use aptos_evm::{
    aptos_events_to_evm_events, make_executor, EvmContext, EvmMachine, EvmState, EvmTransaction,
    ViewWrapper,
};
use aptos_executor::block_executor::BlockExecutor;
use aptos_logger::prelude::*;
use aptos_state_view::TStateView;
use aptos_block_executor::state_view::DbStateView;
use aptos_types::{
    contract_event::ContractEvent,
    transaction::{
        aptos_address_to_eth_address, EthAddress, SignedTransaction, Transaction, TransactionInfo
    },
};
use aptos_block_executor::{
    data_cache::AsMoveResolver, evm_context_loader::ContextView, logging::AdapterLogSchema, aptos_vm::AptosVM,
};
use cfx_evm::{ExecutionOutcome, TransactOptions};
use cfx_primitives::{
    transaction::eip155_signature, Action, SignedTransaction as EthSignedTransaction,
    TransactionWithSignature as EthTransaction,
};
use cfx_state::{state_trait::StateOpsTrait, CleanupMode};
use cfx_types::AddressSpaceUtil;
use ethereum_types::{Bloom, H160, H256, U256, U64};
use jsonrpsee::core::{async_trait, Error, RpcResult};
use rlp::Encodable;
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use pprof::protos::Message;
use aptos_crypto::HashValue;
use aptos_executor_types::BlockExecutorTrait;
use aptos_types::transaction::Transaction::UserTransaction;
use aptos_types::{
    aggregate_signature::AggregateSignature,
    block_info::BlockInfo,
    ledger_info::{LedgerInfo, LedgerInfoWithSignatures},
    transaction::Version,
};

// fn type_of<T>(_: T) -> &'static str {
//     type_name::<T>()
// }

pub(crate) fn gen_li_with_sigs(
    block_id: HashValue,
    root_hash: HashValue,
    version: Version,
) -> LedgerInfoWithSignatures {
    let block_info = BlockInfo::new(
        1,        /* epoch */
        0,        /* round, doesn't matter */
        block_id, /* id, doesn't matter */
        root_hash, version, 0,    /* timestamp_usecs, doesn't matter */
        None, /* next_epoch_state */
    );
    let ledger_info = LedgerInfo::new(
        block_info,
        HashValue::zero(), /* consensus_data_hash, doesn't matter */
    );
    LedgerInfoWithSignatures::new(
        ledger_info,
        AggregateSignature::empty(), /* signatures */
    )
}



pub struct EthHandler {
    context: Context,
    evm_machine: EvmMachine,
}

impl EthHandler {
    pub fn new(context: Context) -> Self {
        Self {
            context,
            evm_machine: EvmMachine::new(),
        }
    }
}

#[async_trait]
impl EthServer for EthHandler {
    async fn net_version(&self) -> RpcResult<String> {
        let id = self.context.evm_chain_id();
        Ok(format!("{}", id))
    }

    async fn max_priority_fee(&self) -> RpcResult<U256> {
        Ok(U256::from(1))
    }

    async fn stress_test_eth_txs_uniswap(&self) -> RpcResult<U256> {
        let _result = self.context.benchmark_mutex.lock().await;
        info!("Running experiment using experimental Ethereum JSON RPC");
        let bytes = std::fs::read("./api/ethrpc/src/impls/transactions.txt").unwrap();
        let mut txs: Vec<Transaction> = Vec::with_capacity(100000);
        let mut current_byte: usize = 0;
        while current_byte < bytes.len() {
            let first_byte = (bytes[current_byte + 1] as u32) << 8;
            let tx_length: u32 = (first_byte + bytes[current_byte] as u32) as u32;
            // println!("Read tx {} with size {}", counter, tx_length);
            current_byte += 2;
            let tx: &[u8] = &bytes[current_byte..current_byte + tx_length as usize];
            let eth_tx: EthTransaction = rlp::decode(tx).map_err(Error::to_call_error)?;
            current_byte += tx_length as usize;
            let signed_tx = SignedTransaction::try_from(eth_tx)?;
            // TODO: add tx type change depending on the appended bits after the tx 
            // signed_tx.transaction_type = ??????; 
            txs.push(UserTransaction(signed_tx));
        }
        let executor: BlockExecutor<AptosVM> = BlockExecutor::new(Arc::try_unwrap(self.context.db.clone()).unwrap_or_else(|arc| (*arc).clone()));
        {
            let guard = pprof::ProfilerGuardBuilder::default().frequency(1000).blocklist(&["libc", "libgcc", "pthread", "vdso"]).build().unwrap();

            // get latest version
         
            for block in txs.chunks(2500) {
                let block_id = HashValue::random();
                let mut block_fixed = block.to_vec();
                block_fixed.push(Transaction::StateCheckpoint(HashValue::random()));

                let version = self
                .context
                .db
                .reader.get_latest_ledger_info()?
                .ledger_info()
                .version();
                let block_length = block_fixed.len() as u64;
                info!("block size {}, current version {}", block_length, version);
                let output = executor.execute_block((block_id, block_fixed), executor.committed_block_id()).expect("TODO: panic message");
                let ledger_info_with_sigs = gen_li_with_sigs(block_id, output.root_hash(), version + block_length);
                executor
                    .commit_blocks(vec![block_id], ledger_info_with_sigs)
                    .unwrap();
            }
           

            // executor.committed_block_id();

            if let Ok(report) = guard.report().build() {
                println!("report time: {}", report.timing.duration.as_secs_f64());
                let file = File::create("flamegraph.svg").unwrap();
                report.flamegraph(file).unwrap();
            };

            if let Ok(report) = guard.report().build() {
                let mut file = File::create("profile.pb").unwrap();
                let profile = report.pprof().unwrap();

                let mut content = Vec::new();
                profile.encode(&mut content).unwrap();
                file.write_all(&content).unwrap();
            };
        }
        Ok(U256::from(1))
    }

    async fn send_raw_transaction(&self, bytes: HexEncodedBytes) -> RpcResult<H256> {
        let eth_tx: EthTransaction = rlp::decode(bytes.inner()).map_err(Error::to_call_error)?;
        let tx_hash = eth_tx.hash();
        // println!("{}", type_of(&eth_tx));
        if let Err(e) = self.context.submit_transaction(eth_tx.try_into()?).await {
            info!("eth submit tx error: e={:?}", e);
            return Err(e.into());
        }
        Ok(tx_hash)
    }

    async fn transaction_receipt(&self, tx_hash: H256) -> RpcResult<Option<Receipt>> {
        match self.transaction_with_info_by_hash(tx_hash)? {
            None => Ok(None),
            Some((eth_tx, info, events)) => {
                // TODO(lpl): Check if we need to double-check the sender in `user_tx`.
                let tx_exec_error_msg = if info.status().is_success() {
                    None
                } else {
                    Some(serde_json::to_string(info.status())?)
                };
                let events = aptos_events_to_evm_events(events)?;
                let mut logs_bloom = Bloom::default();
                for log in &events {
                    logs_bloom.accrue_bloom(&log.bloom());
                }
                Ok(Some(Receipt {
                    transaction_hash: tx_hash,
                    logs: events
                        .into_iter()
                        .map(|e| Log::try_from(e).map_err(Error::from))
                        .collect::<RpcResult<Vec<Log>>>()?,
                    // TODO(lpl): Should Move transactions be counted?
                    transaction_index: eth_tx.transaction_index.expect("executed"),
                    block_hash: eth_tx.block_hash.expect("executed"),
                    from: eth_tx.from,
                    to: eth_tx.to,
                    block_number: eth_tx.block_number.expect("executed"),
                    // TODO(lpl): Is this needed?
                    cumulative_gas_used: Default::default(),
                    gas_used: info.gas_used().into(),
                    contract_address: eth_tx.creates,
                    logs_bloom,
                    // TODO(lpl): Check if all cases are covered.
                    status_code: eth_tx.status.expect("executed"),
                    effective_gas_price: eth_tx.gas_price,
                    // TODO(lpl): Make error msg compatible.
                    tx_exec_error_msg,
                }))
            },
        }
    }

    async fn balance(
        &self,
        address: H160,
        maybe_block_number: Option<BlockNumber>,
    ) -> RpcResult<U256> {
        let state_version = self.get_version_at_block_number(maybe_block_number)?;
        let state_view = self.context.db.reader.state_view_at_version(Some(state_version))?;
        let mut view_wrapper = ViewWrapper {
            inner: &state_view.as_move_resolver(),
            cache: Default::default(),
        };
        let state = EvmState::new(&mut view_wrapper);
        state
            .state
            .balance(&address.with_evm_space())
            .map_err(|e| Error::Custom(e.to_string()))
    }

    async fn call(
        &self,
        mut request: CallRequest,
        maybe_block_number: Option<BlockNumber>,
    ) -> RpcResult<HexEncodedBytes> {
        let state_version = self.get_version_at_block_number(maybe_block_number)?;
        let state_view = self.context.db.reader.state_view_at_version(Some(state_version))?;
        let context = self.get_evm_context(&state_view);
        let mut view_wrapper = ViewWrapper {
            inner: &state_view.as_move_resolver(),
            cache: Default::default(),
        };
        let mut state = EvmState::new(&mut view_wrapper);

        if request.from.is_none() {
            let random_address = EthAddress::random();
            state
                .state
                .add_balance(
                    &random_address.with_evm_space(),
                    &(U256::from(5_000_000_000u64) * U256::from(1_000_000_000_000_000_000u64)),
                    CleanupMode::ForceCreate,
                    0.into(),
                )
                .map_err(|e| Error::Custom(e.to_string()))?;
            request.from = Some(random_address);
        }
        if request.nonce.is_none() {
            request.nonce = Some(
                state
                    .state
                    .nonce(&request.from.unwrap().with_evm_space())
                    .map_err(|e| Error::Custom(e.to_string()))?,
            );
        }
        let eth_tx = sign_call(self.context.evm_chain_id() as u32, request)?;

        let mut executor = make_executor(&self.evm_machine, &context, &mut state);
        let mut options = TransactOptions::exec_with_no_tracing();
        options.check_settings.real_execution = false;
        let output = executor
            .transact(
                &EvmTransaction::try_from(&SignedTransaction::try_from(eth_tx)?).expect("eth tx"),
                options,
            )
            .map_err(|e| Error::Custom(e.to_string()))?;
        match output {
            ExecutionOutcome::Finished(executed) => Ok(executed.output.into()),
            e => Err(Error::Custom(format!("{:?}", e))),
        }
    }

    async fn estimate_gas(
        &self,
        _request: CallRequest,
        _maybe_block_number: Option<BlockNumber>,
    ) -> RpcResult<U256> {
        // FIXME(lpl)
        Ok(3_000_000.into()) // magic constant?
    }

    async fn chain_id(&self) -> RpcResult<Option<U64>> {
        Ok(Some(self.context.evm_chain_id().into()))
    }

    async fn block_number(&self) -> RpcResult<U256> {
        let version = self
            .context
            .db
            .reader.get_latest_ledger_info()?
            .ledger_info()
            .version();
        let (_, _, new_block_event) = self.context.db.reader.get_block_info_by_version(version)?;
        Ok(new_block_event.height().into())
    }

    async fn block_by_number(
        &self,
        block_number: BlockNumber,
        include_txs: bool,
    ) -> RpcResult<Option<Block>> {
        let version = self.get_version_at_block_number(Some(block_number))?;
        self.block_by_version(version, include_txs)
    }

    async fn block_by_hash(&self, block_hash: H256, include_txs: bool) -> RpcResult<Option<Block>> {
        match self
            .context
            .db
            .reader.get_block_version_by_hash(block_hash.into())?
        {
            None => Ok(None),
            Some(version) => self.block_by_version(version, include_txs),
        }
    }

    async fn accounts(&self) -> RpcResult<Vec<H160>> {
        Ok(vec![])
    }

    async fn code_at(
        &self,
        address: H160,
        maybe_block_number: Option<BlockNumber>,
    ) -> RpcResult<HexEncodedBytes> {
        let address = address.with_evm_space();
        let state_version = self.get_version_at_block_number(maybe_block_number)?;
        let state_view = self.context.db.reader.state_view_at_version(Some(state_version))?;
        let mut view_wrapper = ViewWrapper {
            inner: &state_view.as_move_resolver(),
            cache: Default::default(),
        };
        let state = EvmState::new(&mut view_wrapper);
        let code = match state
            .state
            .code(&address)
            .map_err(|e| Error::Custom(e.to_string()))?
        {
            Some(c) => (*c).clone(),
            None => vec![],
        };
        Ok(code.into())
    }

    async fn transaction_count(
        &self,
        address: H160,
        maybe_block_number: Option<BlockNumber>,
    ) -> RpcResult<U256> {
        let address = address.with_evm_space();
        let state_version = self.get_version_at_block_number(maybe_block_number)?;
        let state_view = self.context.db.reader.state_view_at_version(Some(state_version))?;
        let mut view_wrapper = ViewWrapper {
            inner: &state_view.as_move_resolver(),
            cache: Default::default(),
        };
        let state = EvmState::new(&mut view_wrapper);
        state
            .state
            .nonce(&address)
            .map_err(|e| Error::Custom(e.to_string()))
    }

    async fn gas_price(&self) -> RpcResult<U256> {
        Ok(1.into())
    }

    async fn transaction_by_hash(&self, h: H256) -> RpcResult<Option<RpcTransaction>> {
        Ok(self.transaction_with_info_by_hash(h)?.map(|(tx, _, _)| tx))
    }
}

impl EthHandler {
    fn get_version_at_block_number(
        &self,
        maybe_block_number: Option<BlockNumber>,
    ) -> anyhow::Result<Version> {
        let state_version = match maybe_block_number {
            None
            | Some(
                BlockNumber::Latest
                | BlockNumber::Finalized
                | BlockNumber::Pending
                | BlockNumber::Safe,
            ) => self
                .context
                .db
                .reader.get_latest_ledger_info()?
                .ledger_info()
                .version(),
            Some(BlockNumber::Earliest) => 0,
            Some(BlockNumber::Num(block_number)) => {
                let (_, end_version, _) = self.context.db.reader.get_block_info_by_height(block_number)?;
                end_version
            },
            Some(BlockNumber::Hash { .. }) => {
                todo!()
            },
        };
        Ok(state_version)
    }

    fn get_evm_context(&self, state_view: &DbStateView) -> EvmContext {
        let log_context = AdapterLogSchema::new(state_view.id(), 0);
        let vm = AptosVM::new(&state_view);
        let data_cache = state_view.as_move_resolver();
        let context_reader = ContextView::new(&vm, &data_cache, &log_context);
        self.evm_machine.make_context(&context_reader)
    }

    fn block_by_version(&self, version: Version, include_txs: bool) -> RpcResult<Option<Block>> {
        let ledger_version = self
            .context
            .db
            .reader.get_latest_ledger_info()?
            .ledger_info()
            .version();
        let (start_version, end_version, new_block_event) =
            self.context.db.reader.get_block_info_by_version(version)?;
        let block_hash: H256 = new_block_event.hash()?.into();
        let transaction_list = self.context.db.reader.get_transactions(
            start_version,
            end_version - start_version + 1,
            ledger_version,
            false,
        )?;

        let txs_with_status: Vec<_> = transaction_list
            .transactions
            .into_iter()
            .zip(transaction_list.proof.transaction_infos)
            .filter_map(|(tx, info)| match tx {
                Transaction::UserTransaction(user_tx) => user_tx
                    .eth_transaction()
                    .map(|eth_tx| (eth_tx, info.status().is_success())),
                _ => None,
            })
            .collect();
        let block_transactions = if include_txs {
            BlockTransactions::Full(
                txs_with_status
                    .into_iter()
                    .enumerate()
                    .map(|(index, (tx, is_success))| {
                        let public = tx.recover_public().map_err(Error::to_call_error)?;
                        let signed_tx = EthSignedTransaction::new(public, tx);
                        let block_info = (
                            Some(block_hash),
                            Some(new_block_event.height().into()),
                            Some(index.into()),
                        );
                        let exec_info = if is_success {
                            (Some(1.into()), deployed_contract_address(&signed_tx))
                        } else {
                            (Some(0.into()), None)
                        };
                        Ok(RpcTransaction::from_signed(
                            &signed_tx, block_info, exec_info,
                        ))
                    })
                    .collect::<RpcResult<_>>()?,
            )
        } else {
            BlockTransactions::Hashes(
                txs_with_status
                    .into_iter()
                    .map(|(tx, _)| tx.hash())
                    .collect(),
            )
        };
        let block = Block {
            hash: block_hash,
            parent_hash: Default::default(),
            uncles_hash: Default::default(),
            author: aptos_address_to_eth_address(&new_block_event.proposer()),
            miner: aptos_address_to_eth_address(&new_block_event.proposer()),
            state_root: Default::default(),
            transactions_root: Default::default(),
            receipts_root: Default::default(),
            number: new_block_event.height().into(),
            gas_used: Default::default(),
            gas_limit: Default::default(),
            extra_data: vec![].into(),
            logs_bloom: Default::default(),
            timestamp: new_block_event.proposed_time().into(),
            difficulty: Default::default(),
            total_difficulty: Default::default(),
            base_fee_per_gas: None,
            uncles: vec![],
            transactions: block_transactions,
            size: Default::default(),
            nonce: Default::default(),
            mix_hash: Default::default(),
        };
        Ok(Some(block))
    }

    fn transaction_with_info_by_hash(
        &self,
        tx_hash: H256,
    ) -> anyhow::Result<Option<(RpcTransaction, TransactionInfo, Vec<ContractEvent>)>> {
        let ledger_info = self.context.db.reader.get_latest_ledger_info()?;
        match self.context.db.reader.get_transaction_by_eth_hash(
            tx_hash.into(),
            ledger_info.ledger_info().version(),
            true, /* fetch_events */
        )? {
            Some(tx) => {
                let (block_start_version, _, block_info) =
                    self.context.db.reader.get_block_info_by_version(tx.version)?;

                let user_tx = tx.transaction.as_signed_user_txn()?;
                let eth_tx = user_tx
                    .eth_transaction()
                    .ok_or(anyhow::anyhow!("eth hash with an incompatible tx type"))?;
                let public = eth_tx.recover_public().map_err(Error::to_call_error)?;
                let eth_tx = EthSignedTransaction::new(public, eth_tx);
                trace!("eth_tx is {:?}", eth_tx);
                // TODO(lpl): Check if we need to double-check the sender in `user_tx`.
                let from = eth_tx.sender;
                let to = match eth_tx.action() {
                    Action::Create => None,
                    Action::Call(address) => Some(*address),
                };
                let contract_address;
                let status_code;
                if tx.proof.transaction_info.status().is_success() {
                    contract_address = deployed_contract_address(&eth_tx);
                    status_code = 1.into();
                } else {
                    contract_address = None;
                    status_code = 0.into();
                }
                let rpc_tx = RpcTransaction {
                    hash: tx_hash,
                    // TODO(lpl): Should Move transactions be counted?
                    transaction_index: Some((tx.version - block_start_version).into()),
                    block_hash: Some(block_info.hash()?.into()),
                    from,
                    to,
                    value: *eth_tx.value(),
                    gas_price: *eth_tx.gas_price(),
                    max_fee_per_gas: None,
                    gas: *eth_tx.gas(),
                    input: eth_tx.data().clone().into(),
                    creates: contract_address,
                    raw: eth_tx.transaction.transaction.rlp_bytes().to_vec().into(),
                    public_key: eth_tx.public().clone(),
                    chain_id: eth_tx.chain_id().map(Into::into),
                    standard_v: Some(eth_tx.signature().v().into()),
                    v: eip155_signature::add_chain_replay_protection(
                        eth_tx.signature().v(),
                        eth_tx.chain_id().map(|x| x as u64),
                    )
                    .into(),
                    r: eth_tx.signature().r().into(),
                    s: eth_tx.signature().s().into(),
                    block_number: Some(block_info.height().into()),
                    nonce: *eth_tx.nonce(),
                    status: Some(status_code),
                };
                Ok(Some((
                    rpc_tx,
                    tx.proof.transaction_info,
                    tx.events.expect("fetch is true"),
                )))
            },
            None => {
                Ok(None)
            },
        }
    }
}
