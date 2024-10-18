// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

use crate::{
    block_executor::{vm_wrapper::AptosExecutorTaskArc, AptosTransactionOutput}, cached_state_view::CachedStateView, counters, errors2::{Error2, Result2}, output_delta_resolver::OutputDeltaResolver, scheduler::{Scheduler, SchedulerTask, TaskGuard, TxnIndex, Version}, task::{ExecutionStatus, ExecutorTask, Transaction, TransactionOutput}, txn_last_input_output::TxnLastInputOutput, view::{LatestView, MVHashMapView}
};
use aptos_logger::debug;
use aptos_mvhashmap::{MVHashMap, MVHashMapError, MVHashMapOutput};
use aptos_state_view::TStateView;
use aptos_types::{account_address::AccountAddress, block_metadata::BlockMetadata, state_store::state_key::StateKey, transaction::{SignatureCheckedTransaction, WriteSetPayload}, vm_status::VMStatus, write_set::WriteOp};
use cfx_primitives::transaction::TxType;
use num_cpus;
use once_cell::sync::Lazy;
use std::{collections::{btree_map::BTreeMap, HashMap}, hash::Hash, marker::PhantomData, sync::{mpsc::{Receiver, Sender}, Arc, RwLock}};

/// Transactions after signature checking:
/// Waypoints and BlockPrologues are not signed and are unaffected by signature checking,
/// but a user transaction or writeset transaction is transformed to a SignatureCheckedTransaction.
#[derive(Debug, Clone)]
pub enum PreprocessedTransaction {
    UserTransaction(Box<SignatureCheckedTransaction>),
    WaypointWriteSet(WriteSetPayload),
    BlockMetadata(BlockMetadata),
    InvalidSignature,
    StateCheckpoint,
}

pub struct ExecutionStatusMultiWorker {
    pub status: ExecutionStatus<AptosTransactionOutput, VMStatus>,
    pub worker_type: WorkerType,
}

pub type ManagerSender = Sender<ExecutionContext>;
pub type WorkerReceiver = Receiver<ExecutionContext>;

pub type WorkerSender = Sender<Vec<ExecutionStatusMultiWorker>>;
pub type ManagerReceiver = Receiver<Vec<ExecutionStatusMultiWorker>>;

pub type TwoWayChannelEndpoints<'a> = (ManagerSender, &'a ManagerReceiver);

pub struct TwoWayChannels<'a> {
    pub eth_channel: TwoWayChannelEndpoints<'a>,
    pub move_channel: TwoWayChannelEndpoints<'a>,
    pub cross_channel: TwoWayChannelEndpoints<'a>
}

impl Transaction for PreprocessedTransaction {
    type Key = StateKey;
    type Value = WriteOp;
}

#[derive(PartialEq, Eq, Hash, Debug)]
pub enum ExecutionSpace {
    Aptos = 0,
    Ethereum = 1
}

#[derive(Debug, PartialEq, Eq, Hash, Copy, Clone)]
pub enum TxScanPointer {
    Aptos = 0,
    Ethereum = 1,
    Cross = 2
}

pub type WorkerType = TxScanPointer;

pub struct ExecutionContext
{
    pub base_view: Arc<CachedStateView>,
    pub executor: Arc<AptosExecutorTaskArc<CachedStateView>>, 
    pub execution_state_cache: Arc<RwLock<BTreeMap<StateKey, WriteOp>>>, 
    pub idx: TxnIndex,
    pub txn_batch: Vec<PreprocessedTransaction>
}

impl ExecutionContext
{
    fn new(
        base_view: &Arc<CachedStateView>, 
        executor: &Arc<AptosExecutorTaskArc<CachedStateView>>, 
        execution_state_cache: &Arc<RwLock<BTreeMap<StateKey, WriteOp>>>, 
        idx: &TxnIndex, 
        txn_batch: Vec<PreprocessedTransaction>
    ) -> Self {
        ExecutionContext {
            base_view: base_view.clone(),
            executor: executor.clone(),
            execution_state_cache: execution_state_cache.clone(),
            idx: *idx,
            txn_batch: txn_batch,
        }
    }
}

pub static RAYON_EXEC_POOL: Lazy<rayon::ThreadPool> = Lazy::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(num_cpus::get())
        .thread_name(|index| format!("par_exec_{}", index))
        .build()
        .unwrap()
});

pub struct BlockExecutor<T, E, S> {
    // number of active concurrent tasks, corresponding to the maximum number of rayon
    // threads that may be concurrently participating in parallel execution.
    concurrency_level: usize,
    phantom: PhantomData<(T, E, S)>,
}

impl<T, E, S> BlockExecutor<T, E, S>
where
    T: Transaction,
    E: ExecutorTask<Txn = T>,
    S: TStateView<Key = T::Key> + Sync + Send,
{
    /// The caller needs to ensure that concurrency_level > 1 (0 is illegal and 1 should
    /// be handled by sequential execution) and that concurrency_level <= num_cpus.
    pub fn new(concurrency_level: usize) -> Self {
        assert!(
            concurrency_level > 0 && concurrency_level <= num_cpus::get(),
            "Parallel execution concurrency level {} should be between 1 and number of CPUs",
            concurrency_level
        );
        Self {
            concurrency_level,
            phantom: PhantomData,
        }
    }

    fn execute<'a>(
        &self,
        version: Version,
        guard: TaskGuard<'a>,
        signature_verified_block: &[PreprocessedTransaction],
        last_input_output: &TxnLastInputOutput<T::Key, E::Output, E::Error>,
        versioned_data_cache: &MVHashMap<T::Key, T::Value>,
        scheduler: &'a Scheduler,
        executor: &E,
        base_view: &'a S,
    ) -> SchedulerTask<'a> {
        let (idx_to_execute, incarnation) = version;
        let txn = &signature_verified_block[idx_to_execute];

        let speculative_view = MVHashMapView::new(versioned_data_cache, scheduler);

        // VM execution.
        let execute_result = executor.execute_transaction(
            &LatestView::<T, S>::new_mv_view(base_view, &speculative_view, idx_to_execute),
            txn,
            idx_to_execute,
            false,
        );
        let mut prev_modified_keys = last_input_output.modified_keys(idx_to_execute);

        // For tracking whether the recent execution wrote outside of the previous write/delta set.
        let mut updates_outside = false;
        let mut apply_updates = |output: &<E as ExecutorTask>::Output| {
            // First, apply writes.
            let write_version = (idx_to_execute, incarnation);
            for (k, v) in output.get_writes().into_iter() {
                if !prev_modified_keys.remove(&k) {
                    updates_outside = true;
                }
                versioned_data_cache.add_write(&k, write_version, v);
            }

            // Then, apply deltas.
            for (k, d) in output.get_deltas().into_iter() {
                if !prev_modified_keys.remove(&k) {
                    updates_outside = true;
                }
                versioned_data_cache.add_delta(&k, idx_to_execute, d);
            }
        };

        let result = match execute_result {
            // These statuses are the results of speculative execution, so even for
            // SkipRest (skip the rest of transactions) and Abort (abort execution with
            // user defined error), no immediate action is taken. Instead the statuses
            // are recorded and (final statuses) are analyzed when the block is executed.
            ExecutionStatus::Success(output) => {
                // Apply the writes/deltas to the versioned_data_cache.
                apply_updates(&output);
                ExecutionStatus::Success(output)
            },
            ExecutionStatus::SkipRest(output) => {
                // Apply the writes/deltas and record status indicating skip.
                apply_updates(&output);
                ExecutionStatus::SkipRest(output)
            },
            ExecutionStatus::Abort(err) => {
                // Record the status indicating abort.
                ExecutionStatus::Abort(Error2::UserError(err))
            },
        };

        // Remove entries from previous write/delta set that were not overwritten.
        for k in prev_modified_keys {
            versioned_data_cache.delete(&k, idx_to_execute);
        }

        last_input_output.record(idx_to_execute, speculative_view.take_reads(), result);
        scheduler.finish_execution(idx_to_execute, incarnation, updates_outside, guard)
    }

    fn validate<'a>(
        &self,
        version_to_validate: Version,
        guard: TaskGuard<'a>,
        last_input_output: &TxnLastInputOutput<T::Key, E::Output, E::Error>,
        versioned_data_cache: &MVHashMap<T::Key, T::Value>,
        scheduler: &'a Scheduler,
    ) -> SchedulerTask<'a> {
        use MVHashMapError::*;
        use MVHashMapOutput::*;

        let (idx_to_validate, incarnation) = version_to_validate;
        let read_set = last_input_output
            .read_set(idx_to_validate)
            .expect("Prior read-set must be recorded");

        let valid = read_set.iter().all(|r| {
            match versioned_data_cache.read(r.path(), idx_to_validate) {
                Ok(Version(version, _)) => r.validate_version(version),
                Ok(Resolved(value)) => r.validate_resolved(value),
                Err(Dependency(_)) => false, // Dependency implies a validation failure.
                Err(Unresolved(delta)) => r.validate_unresolved(delta),
                Err(NotFound) => r.validate_storage(),
                // We successfully validate when read (again) results in a delta application
                // failure. If the failure is speculative, a later validation will fail due to
                // a read without this error. However, if the failure is real, passing
                // validation here allows to avoid infinitely looping and instead panic when
                // materializing deltas as writes in the final output preparation state. Panic
                // is also preferrable as it allows testing for this scenario.
                Err(DeltaApplicationFailure) => r.validate_delta_application_failure(),
            }
        });

        let aborted = !valid && scheduler.try_abort(idx_to_validate, incarnation);

        if aborted {
            counters::SPECULATIVE_ABORT_COUNT.inc();

            // Not valid and successfully aborted, mark the latest write/delta sets as estimates.
            for k in last_input_output.modified_keys(idx_to_validate) {
                versioned_data_cache.mark_estimate(&k, idx_to_validate);
            }

            scheduler.finish_abort(idx_to_validate, incarnation, guard)
        } else {
            SchedulerTask::NoTask
        }
    }

    fn work_task_with_scope(
        &self,
        executor_arguments: &E::Argument,
        block: &[PreprocessedTransaction],
        last_input_output: &TxnLastInputOutput<T::Key, E::Output, E::Error>,
        versioned_data_cache: &MVHashMap<T::Key, T::Value>,
        scheduler: &Scheduler,
        base_view: &S,
    ) {
        // Make executor for each task. TODO: fast concurrent executor.
        let executor = E::init(*executor_arguments);

        let mut scheduler_task = SchedulerTask::NoTask;
        loop {
            scheduler_task = match scheduler_task {
                SchedulerTask::ValidationTask(version_to_validate, guard) => self.validate(
                    version_to_validate,
                    guard,
                    last_input_output,
                    versioned_data_cache,
                    scheduler,
                ),
                SchedulerTask::ExecutionTask(version_to_execute, None, guard) => self.execute(
                    version_to_execute,
                    guard,
                    block,
                    last_input_output,
                    versioned_data_cache,
                    scheduler,
                    &executor,
                    base_view,
                ),
                SchedulerTask::ExecutionTask(_, Some(condvar), _guard) => {
                    let (lock, cvar) = &*condvar;
                    // Mark dependency resolved.
                    *lock.lock() = true;
                    // Wake up the process waiting for dependency.
                    cvar.notify_one();

                    SchedulerTask::NoTask
                },
                SchedulerTask::NoTask => scheduler.next_task(),
                SchedulerTask::Done => {
                    break;
                },
            }
        }
    }

    pub fn execute_transactions_parallel(
        &self,
        executor_initial_arguments: E::Argument,
        signature_verified_block: &Vec<PreprocessedTransaction>,
        base_view: &S,
    ) -> Result2<(Vec<E::Output>, OutputDeltaResolver<T::Key, T::Value>), E::Error> {
        assert!(self.concurrency_level > 1, "Must use sequential execution");

        let versioned_data_cache = MVHashMap::new();

        if signature_verified_block.is_empty() {
            return Ok((vec![], OutputDeltaResolver::new(versioned_data_cache)));
        }

        let num_txns = signature_verified_block.len();
        let last_input_output = TxnLastInputOutput::new(num_txns);
        let scheduler = Scheduler::new(num_txns);

        RAYON_EXEC_POOL.scope(|s| {
            for _ in 0..self.concurrency_level {
                s.spawn(|_| {
                    self.work_task_with_scope(
                        &executor_initial_arguments,
                        signature_verified_block,
                        &last_input_output,
                        &versioned_data_cache,
                        &scheduler,
                        base_view,
                    );
                });
            }
        });

        // TODO: for large block sizes and many cores, extract outputs in parallel.
        let num_txns = scheduler.num_txn_to_execute();
        let mut final_results = Vec::with_capacity(num_txns);

        let maybe_err = if last_input_output.module_publishing_may_race() {
            counters::MODULE_PUBLISHING_FALLBACK_COUNT.inc();
            Some(Error2::ModulePathReadWrite)
        } else {
            let mut ret = None;
            for idx in 0..num_txns {
                match last_input_output.take_output(idx) {
                    ExecutionStatus::Success(t) => final_results.push(t),
                    ExecutionStatus::SkipRest(t) => {
                        final_results.push(t);
                        break;
                    },
                    ExecutionStatus::Abort(err) => {
                        ret = Some(err);
                        break;
                    },
                };
            }
            ret
        };

        RAYON_EXEC_POOL.spawn(move || {
            // Explicit async drops.
            drop(last_input_output);
            drop(scheduler);
        });

        match maybe_err {
            Some(err) => Err(err),
            None => {
                final_results.resize_with(num_txns, E::Output::skip_output);
                Ok((
                    final_results,
                    OutputDeltaResolver::new(versioned_data_cache),
                ))
            },
        }
    }
    

    pub fn execute_transactions_parallel_prototype(
        &self,
        signature_verified_block: &[PreprocessedTransaction],
        base_view: Arc<CachedStateView>,
        channels: &TwoWayChannels,
        previous_nonce_per_address: &mut HashMap<AccountAddress, u64>
    ) -> Result<Vec<AptosTransactionOutput>, Error2<VMStatus>> 
    {
        let TwoWayChannels {
            move_channel, 
            eth_channel, 
            cross_channel
        } = channels;

        let block_size = signature_verified_block.len();
        
        let executed_state_cache = Arc::new(RwLock::new(BTreeMap::new()));
        let mut ret = Vec::with_capacity(block_size);
        
        let executor = Arc::new(<AptosExecutorTaskArc<CachedStateView>>::init(base_view.clone()));
       
        let mut processed_transactions = 0;

        const SCANNING_POINTERS_TYPES: [TxScanPointer; 3] = [TxScanPointer::Aptos, TxScanPointer::Ethereum, TxScanPointer::Cross];
        // const SCANNING_POINTERS_TYPES: [TxScanPointer; 1] = [TxScanPointer::Aptos];
    
        let mut pointer_type_to_txn_idx_in_block: [usize; 3] = [0, 0, 0];
        let mut pointer_type_to_whether_worker_is_loaded = [false, false, false];
        let pointer_type_to_channel_endpoints = [move_channel, eth_channel, cross_channel];
        let mut transaction_type_to_scanning_pointer = [TxScanPointer::Ethereum, TxScanPointer::Aptos, TxScanPointer::Cross, TxScanPointer::Cross];       
        
        let mut batch_end_flag;
        let mut tx_batch ;
        let mut pointer_idx: &mut usize;
        let mut sender_handle: &Sender<ExecutionContext>; // default

        while processed_transactions < block_size {
            // println!("{}", processed_transactions);
            for pointer_type in SCANNING_POINTERS_TYPES.iter() {
                
                pointer_idx = &mut pointer_type_to_txn_idx_in_block[*pointer_type as usize];
                tx_batch = Vec::new();
                batch_end_flag = false;

                sender_handle = &cross_channel.0; // default

                while *pointer_idx < block_size && !batch_end_flag {
                    let tx_of_potentially_correct_type = &signature_verified_block[*pointer_idx];
                    let tx_type: &TxType;
                    match tx_of_potentially_correct_type {
                        PreprocessedTransaction::StateCheckpoint => {
                            tx_type = &TxType::CrossTxAptosOrigin;
                        },
                        PreprocessedTransaction::UserTransaction(inner_tx) => {
                            tx_type = self.fetch_tx_type(inner_tx);
                        }
                        _ => {
                            panic!("THIS SHOULD NEVER HAPPEN");
                        }
                    }
                    
                    match tx_of_potentially_correct_type {
                        PreprocessedTransaction::UserTransaction(_) | PreprocessedTransaction::StateCheckpoint => {
                            let check_result: Result<&ManagerSender, ()> = self.check_whether_need_to_wait_or_skip_txn(
                                tx_of_potentially_correct_type,
                                channels,
                                pointer_idx,
                                &mut pointer_type_to_whether_worker_is_loaded,
                                previous_nonce_per_address,
                                *pointer_type,
                                processed_transactions,
                                block_size,
                                &mut transaction_type_to_scanning_pointer,
                                tx_type
                            );
                            
                            if let Ok(handle) = check_result {
                                tx_batch.push(tx_of_potentially_correct_type.clone());
                                sender_handle = handle;
                                *pointer_idx += 1;
                            } else {
                                batch_end_flag = true; // TODO: fix to skip Move/Eth txs for Eth/Move pointers but not break the loop!
                            }
                        }
                        _ => {
                            // debug!("Other type of transaction ({:?}) {:?}", *pointer_type, tx_of_potentially_correct_type);
                            *pointer_idx += 1;
                        }
                    }
                }

                if tx_batch.len() > 0 {
                    self.send_txns_to_worker(
                        tx_batch,
                        &base_view,
                        &executor,
                        &executed_state_cache,
                        pointer_idx,
                        *pointer_type,
                        sender_handle,
                        &mut pointer_type_to_whether_worker_is_loaded,
                    );
                }

              
                let channel_is_loaded = pointer_type_to_whether_worker_is_loaded[*pointer_type as usize];

                if channel_is_loaded {
                    let receiver_handle = pointer_type_to_channel_endpoints[*pointer_type as usize].1;
                    // try fetching the response on the other end of the channel
                    let response = receiver_handle.try_recv();
                    
                    match response {
                        Ok(execution_results) => {
                            for ExecutionStatusMultiWorker {status: res, worker_type} in execution_results {
                            
                                // debug!("Recieved reply from {:?} job queue!", *pointer_type);
                                pointer_type_to_whether_worker_is_loaded[worker_type as usize] = false;

                                match res {
                                    ExecutionStatus::Success(output) | ExecutionStatus::SkipRest(output) => {
                                        assert_eq!(
                                            output.get_deltas().len(),
                                            0,
                                            "Sequential execution must materialize deltas"
                                        );
                                        ret.push(output);
                                    },
                                    ExecutionStatus::Abort(err) => {
                                        // Record the status indicating abort.
                                        return Err(Error2::UserError(err));
                                    },
                                }
        
                                // if let Err(e) = exec_status {
                                    // debug!("Execution status is err: {:?}", e);
                                // }
                                processed_transactions = processed_transactions + 1;
                                // debug!("Previous nonce map: {:?}", previous_nonce_per_address);
                                // debug!("Processed transactions so far from this block: {processed_transactions}/{block_size}");
                                if processed_transactions == block_size {
                                    println!("Block finished! {processed_transactions}/{block_size}");
                                }
                            }
                        }
                        Err(_e) => {
                            // debug!("Error while trying to receive execution results for pointer {pointer_type:?}: {e}");
                        }
                    }
                }
            }
        }

        ret.resize_with(block_size, AptosTransactionOutput::skip_output);
        Ok(ret)
    }

    fn fetch_tx_type<'a>(&self, inner_tx: &'a Box<SignatureCheckedTransaction>) -> &'a TxType {
        &(*(*inner_tx)).transaction_type
    }

    fn check_whether_need_to_wait_or_skip_txn<'a>(
        &'a self,
        txn: &PreprocessedTransaction, 
        channels: &'a TwoWayChannels,
        tx_pointer_idx: &mut usize,
        are_workers_loaded: &mut [bool; 3],
        previous_nonce_per_address:  &mut HashMap<AccountAddress, u64>,
        pointer_type: TxScanPointer,
        processed_transactions: usize,
        block_size: usize,
        transaction_type_to_scanning_pointer: &mut [TxScanPointer; 4],
        tx_type: &TxType,
    ) -> Result<&ManagerSender, ()>
    {
        let TwoWayChannels {
            move_channel, 
            eth_channel, 
            cross_channel
        } = channels;

        let move_worker_is_loaded = &are_workers_loaded[TxScanPointer::Aptos as usize];
        let eth_worker_is_loaded = &are_workers_loaded[TxScanPointer::Ethereum as usize];
        let cross_worker_is_loaded = &are_workers_loaded[TxScanPointer::Cross as usize];

        match txn {
            // send checkpoint transactions to the cross channel, otherwise it never executes :(
            PreprocessedTransaction::StateCheckpoint => {
                if *move_worker_is_loaded || *eth_worker_is_loaded || *cross_worker_is_loaded {
                    // let error_text = String::from("StateCheckpoint: one of the channels is loaded. Early return, waiting for workers to be free again...");
                    // debug!(
                    //     "Ethereum, Move, Cross workers statuses: {}, {}, {}", 
                    //     eth_worker_is_loaded, 
                    //     move_worker_is_loaded, 
                    //     cross_worker_is_loaded
                    // );
                    // debug!("{}", error_text);
                    return Result::Err(());
                } 

                if processed_transactions != block_size - 1 {
                    // let error_text = String::from("StateCheckpoint: waiting until all other transactions in this block will be executed.");
                    return Result::Err(());
                }
                
                if !(matches!(pointer_type, TxScanPointer::Cross)) {
                    *tx_pointer_idx += 1;

                    // let error_text: String = format!("StateCheckpoint: {:?} pointer going through the txn, skipping..", pointer_type);
                    // debug!("{}", error_text);
                    return Result::Err(())
                }


                // no need to check for nonces here since this transaction always comes last in the block
                // println!("Checkpoint tx will be sent to a thread soon..");

                Result::Ok(&cross_channel.0)
            }
            PreprocessedTransaction::UserTransaction(inner_tx) => {
                let current_tx_nonce =  &(*(*inner_tx)).sequence_number();
                let transaction_sender = &(*(*inner_tx)).sender();
                

                let actual_pointer_type = pointer_type;
                let expected_pointer_type = transaction_type_to_scanning_pointer[*tx_type as usize];


                // skip transactions that are not in your category 
                // (e.g. Aptos pointer points to an ETH tx -> skip txn + advance ptr)
                if expected_pointer_type != actual_pointer_type { 
                    // let error_text: String = format!(
                    //     "UserTransaction: {:?} pointer going through transaction with index {} that should be handled by {:?} pointer",  
                    //     actual_pointer_type,
                    //     tx_pointer_idx,
                    //     expected_pointer_type
                    // );
                    // debug!("{}", error_text);
                    *tx_pointer_idx += 1; 
                    return Result::Err(())
                }


                // let loaded_worker_error_text: String = format!(
                //     "UserTransaction: {:?} pointer has to wait since one of the workers is currently busy",  
                //     actual_pointer_type
                // );
                // let nonce_sync_error_text: String = format!(
                //     "UserTransaction: {:?} pointer has to wait since the transaction with next nonce has to be executed by other worker",  
                //     actual_pointer_type
                // );

             
                match tx_type {
                    TxType::EthereumTx => {
                        // debug!("{:?} transaction with nonce {} is a native Eth tx; called for a tx pointer of type {:?}", pointer_type, current_tx_nonce, pointer_type);
                        // println!("ExecutionSpace::Ethereum as usize: {}, last_eth_nonce: {}", ExecutionSpace::Ethereum as usize, last_ethereum_nonce);
                        
                        if *eth_worker_is_loaded || *cross_worker_is_loaded {
                            // debug!("{}", loaded_worker_error_text);
                            // debug!(
                            //     "Ethereum, Cross workers statuses: {}, {}", 
                            //     eth_worker_is_loaded,  
                            //     cross_worker_is_loaded
                            // );
                            return Result::Err(())
                        }
                        

                        if let Some(last_ethereum_nonce) = previous_nonce_per_address.get_mut(transaction_sender) {
                            if *last_ethereum_nonce == *current_tx_nonce - 1 {
                                *last_ethereum_nonce = *current_tx_nonce;
                            } else {
                                return Result::Err(());
                            }
                        } else {
                            if *current_tx_nonce == 0 {
                                previous_nonce_per_address.insert(*transaction_sender, 0);
                            } else {
                                return Result::Err(());
                            }
                        }

                        // debug!("Ethereum tx will soon be sent to the thread...");
                        Result::Ok(&eth_channel.0)
                    },
                    TxType::AptosTx => {
                        // debug!("{:?} transaction with nonce {} is a native Move tx; called for a tx pointer of type {:?}", pointer_type, current_tx_nonce, pointer_type);
                        // println!("ExecutionSpace::Aptos as usize: {}, last_aptos_nonce: {}", ExecutionSpace::Aptos as usize, last_aptos_nonce);
                        if *move_worker_is_loaded || *cross_worker_is_loaded {
                            // debug!("{}", loaded_worker_error_text);
                            // debug!(
                            //     "Move, Cross workers statuses: {}, {}",  
                            //     move_worker_is_loaded, 
                            //     cross_worker_is_loaded
                            // );
                            return Result::Err(())
                        }

                        if let Some(last_aptos_nonce) = previous_nonce_per_address.get_mut(transaction_sender) {
                            if *last_aptos_nonce == *current_tx_nonce - 1 {
                                *last_aptos_nonce = *current_tx_nonce;
                            } else {
                                return Result::Err(());
                            }
                        } else {
                            if *current_tx_nonce == 0 {
                                previous_nonce_per_address.insert(*transaction_sender, 0);
                            } else {
                                return Result::Err(());
                            }
                        }

                        // debug!("Aptos tx will soon be sent to the thread...");
                        Result::Ok(&move_channel.0)
                    },
                    TxType::CrossTxAptosOrigin | TxType::CrossTxEthereumOrigin => {
                        // match tx_type {
                        //      TxType::CrossTxAptosOrigin => {
                        //          debug!("Move transaction with nonce {} is a cross Move tx; called for a tx pointer of type {:?}", current_tx_nonce, pointer_type)
                        //      },
                        //      TxType::CrossTxEthereumOrigin => {
                        //          debug!("Ethereum transaction with nonce {} is a cross Ethereum tx; called for a tx pointer of type {:?}", current_tx_nonce, pointer_type)
                        //      },
                        //      _ => ()
                        // }

                        if *move_worker_is_loaded || *eth_worker_is_loaded || *cross_worker_is_loaded {
                            // debug!("{}", loaded_worker_error_text);
                            // debug!(
                            //     "Ethereum, Move, Cross workers statuses: {}, {}, {}", 
                            //     eth_worker_is_loaded, 
                            //     move_worker_is_loaded, 
                            //     cross_worker_is_loaded
                            // );

                            return Result::Err(())
                        }

                 
                        if let Some(last_nonce) = previous_nonce_per_address.get_mut(transaction_sender) {
                            if *last_nonce == *current_tx_nonce - 1 {
                                *last_nonce = *current_tx_nonce;
                            } else {
                                // debug!("{}", nonce_sync_error_text);
                                return Result::Err(());
                            }
                        } else {
                            if *current_tx_nonce == 0 {
                                previous_nonce_per_address.insert(*transaction_sender, 0);
                            } else {
                                return Result::Err(());
                            }
                        }


                        // debug!("Cross tx will soon be sent to the thread...");
                        Result::Ok(&cross_channel.0)
                    },
                    TxType::PlaceholderTypeTx => {
                        debug!("Placeholder txn!!!");
                        panic!("SHOULD NOT EXECUTE: Placeholder Txn");
                    }
                }
            },
            _ => Result::Err(()) // SHOULD NOT EXECUTE: Other type of Txn
        }
    }

    fn send_txns_to_worker(
        &self,
        txn_batch: Vec<PreprocessedTransaction>,
        base_view: &Arc<CachedStateView>,
        executor: &Arc<AptosExecutorTaskArc<CachedStateView>>,
        executed_state_cache: &Arc<RwLock<BTreeMap<StateKey, WriteOp>>>, 
        tx_pointer_idx: &mut usize,
        pointer_type: TxScanPointer,
        sender_handle: &ManagerSender,
        are_workers_loaded: &mut [bool; 3],
    ) {

        match txn_batch.last().unwrap() {
            PreprocessedTransaction::UserTransaction(_) => {
                // invariant: once here, safe to send for execution 
                let request_status = sender_handle.send(
                    ExecutionContext::new(
                        base_view,
                        executor,
                        executed_state_cache,
                        tx_pointer_idx,
                        txn_batch
                    )
                );

                if let Err(e) = request_status {
                    match pointer_type {
                        TxScanPointer::Ethereum => panic!("Ethereum channel stopped accepting tx processing requests... Error: {e:?}"),
                        TxScanPointer::Aptos => panic!("Aptos channel stopped accepting tx processing requests... Error: {e:?}"),
                        TxScanPointer::Cross => panic!("Cross channel stopped accepting tx processing requests... Error: {e:?}"),
                        _ => panic!("SHOULD NEVER EXECUTE")
                    }
                } else {
                    match pointer_type {
                        TxScanPointer::Ethereum => {
                            // debug!("Successful delivery of an Ethereum tx!");
                            are_workers_loaded[TxScanPointer::Ethereum as usize] = true;
                        },
                        TxScanPointer::Aptos => {
                            // debug!("Successful delivery of an Aptos tx!");
                            are_workers_loaded[TxScanPointer::Aptos as usize] = true;
                        }
                        TxScanPointer::Cross => {
                            // debug!("Successful delivery of a Cross tx!");
                            are_workers_loaded[TxScanPointer::Cross as usize] = true;
                        },
                        _ => panic!("SHOULD NEVER EXECUTE")
                    }
                    
                }
            },
            PreprocessedTransaction::StateCheckpoint => {
                let request_status = sender_handle.send(
                    ExecutionContext::new(
                        base_view,
                        executor,
                        executed_state_cache,
                        tx_pointer_idx,
                        txn_batch
                    )
                );
                
                if let Err(e) = request_status {
                    panic!("Cross handler stopped accepting tx processing requests... Error: {:?}", e);
                } else {
                    // debug!("Successful delivery of a checkpoint tx!");
                    are_workers_loaded[TxScanPointer::Cross as usize] = true;
                }
            }
            _ => ()
        }
       
    }

    // ExecutionProxy = actual top-level type of the executor
    // T = PreprocessedTransaction, E = AptosExecutorTask<S>, S = CachedStateView
    pub fn execute_transactions_sequential(
        &self,
        executor_arguments: E::Argument,
        signature_verified_block: &[PreprocessedTransaction],
        base_view: &S,
    ) -> Result2<Vec<E::Output>, E::Error> {
        let num_txns = signature_verified_block.len();
        let executor = E::init(executor_arguments);
        let mut data_map = BTreeMap::new();  
        let mut ret = Vec::new();      
        
        /*
        pub enum TxType { 
            CrossTxEthereumOrigin,
            PlaceholderTypeTx    
        }
        */
        for (idx, txn) in signature_verified_block.iter().enumerate() {
            let res: ExecutionStatus<<E as ExecutorTask>::Output, <E as ExecutorTask>::Error> = executor.execute_transaction(
                &LatestView::<T, S>::new_btree_view(base_view, &data_map, idx),
                txn,
                idx,
                true,
            );

            
            let must_skip = matches!(res, ExecutionStatus::SkipRest(_));

            match res {
                ExecutionStatus::Success(output) | ExecutionStatus::SkipRest(output) => {
                    assert_eq!(
                        output.get_deltas().len(),
                        0,
                        "Sequential execution must materialize deltas"
                    );
                    // Apply the writes.
                    for (ap, write_op) in output.get_writes().into_iter() {
                        data_map.insert(ap, write_op);
                    }
                    ret.push(output);
                },
                ExecutionStatus::Abort(err) => {
                    // Record the status indicating abort.
                    return Err(Error2::UserError(err));
                },
            }

            if must_skip {
                break;
            }
        }

        ret.resize_with(num_txns, E::Output::skip_output);
        Ok(ret)

    }
}
