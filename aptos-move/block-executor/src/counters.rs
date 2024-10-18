// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

use aptos_metrics_core::{register_histogram, register_int_counter, register_int_counter_vec, Histogram, IntCounter, IntCounterVec};
use once_cell::sync::Lazy;


/// Count the number of transactions that brake invariants of VM.
pub static TRANSACTIONS_INVARIANT_VIOLATION: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "aptos_vm_transactions_invariant_violation",
        "Number of transactions that broke VM invariant",
    )
    .unwrap()
});

/// Count the number of transactions validated, with a "status" label to
/// distinguish success or failure results.
pub static TRANSACTIONS_VALIDATED: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "aptos_vm_transactions_validated",
        "Number of transactions validated",
        &["status"]
    )
    .unwrap()
});

/// Count the number of user transactions executed, with a "status" label to
/// distinguish completed vs. discarded transactions.
pub static USER_TRANSACTIONS_EXECUTED: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "aptos_vm_user_transactions_executed",
        "Number of user transactions executed",
        &["status"]
    )
    .unwrap()
});

/// Count the number of system transactions executed.
pub static SYSTEM_TRANSACTIONS_EXECUTED: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "aptos_vm_system_transactions_executed",
        "Number of system transactions executed"
    )
    .unwrap()
});

pub static BLOCK_TRANSACTION_COUNT: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "aptos_vm_num_txns_per_block",
        "Number of transactions per block"
    )
    .unwrap()
});

pub static TXN_TOTAL_SECONDS: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "aptos_vm_txn_total_seconds",
        "Execution time per user transaction"
    )
    .unwrap()
});

pub static TXN_VALIDATION_SECONDS: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "aptos_vm_txn_validation_seconds",
        "Validation time per user transaction"
    )
    .unwrap()
});

pub static TXN_GAS_USAGE: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!("aptos_vm_txn_gas_usage", "Gas used per transaction").unwrap()
});

/// Count the number of critical errors. This is not intended for display
/// on a dashboard but rather for triggering alerts.
pub static CRITICAL_ERRORS: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!("aptos_vm_critical_errors", "Number of critical errors").unwrap()
});


/// Count of times the module publishing fallback was triggered in parallel execution.
pub static MODULE_PUBLISHING_FALLBACK_COUNT: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "aptos_execution_module_publishing_fallback_count",
        "Count times module was read and written in parallel execution (sequential fallback)"
    )
    .unwrap()
});

/// Count of speculative transaction re-executions due to a failed validation.
pub static SPECULATIVE_ABORT_COUNT: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "aptos_execution_speculative_abort_count",
        "Number of speculative aborts in parallel execution (leading to re-execution)"
    )
    .unwrap()
});

/// Count of times a transaction got suspended due to an estimated r/w dependency.
pub static DEPENDENCY_SUSPEND_COUNT: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "aptos_execution_dependency_suspend_count",
        "Count write estimates encountered when reading in parallel execution \
        (typically leading to a suspension / waiting)"
    )
    .unwrap()
});
