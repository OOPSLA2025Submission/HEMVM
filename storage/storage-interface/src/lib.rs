// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0


use aptos_crypto::{hash::CryptoHash, HashValue};
use aptos_types::{
    ledger_info::LedgerInfoWithSignatures,
    state_store::{
        state_key::StateKey,
        state_value::{StateValue},
    },
    transaction::{
        TransactionToCommit, Version,
    },
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;

pub mod async_proof_fetcher;
mod metrics;
#[cfg(any(test, feature = "fuzzing"))]
pub mod mock;
pub mod sync_proof_fetcher;

use aptos_block_executor::{state_view::{DbReader, DbWriter}};

// This is last line of defense against large queries slipping through external facing interfaces,
// like the API and State Sync, etc.
pub const MAX_REQUEST_LIMIT: u64 = 10000;



#[derive(Debug, Deserialize, Error, PartialEq, Eq, Serialize)]
pub enum Error {
    #[error("Service error: {:?}", error)]
    ServiceError { error: String },

    #[error("Serialization error: {0}")]
    SerializationError(String),
}

impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        Self::ServiceError {
            error: format!("{}", error),
        }
    }
}

impl From<bcs::Error> for Error {
    fn from(error: bcs::Error) -> Self {
        Self::SerializationError(format!("{}", error))
    }
}

impl From<aptos_secure_net::Error> for Error {
    fn from(error: aptos_secure_net::Error) -> Self {
        Self::ServiceError {
            error: format!("{}", error),
        }
    }
}




#[derive(Clone)]
pub struct DbReaderWriter {
    pub reader: Arc<dyn DbReader>,
    pub writer: Arc<dyn DbWriter>,
}

impl DbReaderWriter {
    pub fn new<D: 'static + DbReader + DbWriter>(db: D) -> Self {
        let reader = Arc::new(db);
        let writer = Arc::clone(&reader);

        Self { reader, writer }
    }

    pub fn from_arc<D: 'static + DbReader + DbWriter>(arc_db: Arc<D>) -> Self {
        let reader = Arc::clone(&arc_db);
        let writer = Arc::clone(&arc_db);

        Self { reader, writer }
    }

    pub fn wrap<D: 'static + DbReader + DbWriter>(db: D) -> (Arc<D>, Self) {
        let arc_db = Arc::new(db);
        (Arc::clone(&arc_db), Self::from_arc(arc_db))
    }
}

/// Network types for storage service
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum StorageRequest {
    GetStateValueByVersionRequest(Box<GetStateValueByVersionRequest>),
    GetStartupInfoRequest,
    SaveTransactionsRequest(Box<SaveTransactionsRequest>),
}

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
pub struct GetStateValueByVersionRequest {
    /// The access key for the resource
    pub state_key: StateKey,

    /// The version the query is based on.
    pub version: Version,
}

impl GetStateValueByVersionRequest {
    /// Constructor.
    pub fn new(state_key: StateKey, version: Version) -> Self {
        Self { state_key, version }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SaveTransactionsRequest {
    pub txns_to_commit: Vec<TransactionToCommit>,
    pub first_version: Version,
    pub ledger_info_with_signatures: Option<LedgerInfoWithSignatures>,
}

impl SaveTransactionsRequest {
    /// Constructor.
    pub fn new(
        txns_to_commit: Vec<TransactionToCommit>,
        first_version: Version,
        ledger_info_with_signatures: Option<LedgerInfoWithSignatures>,
    ) -> Self {
        SaveTransactionsRequest {
            txns_to_commit,
            first_version,
            ledger_info_with_signatures,
        }
    }
}

pub fn jmt_updates(
    state_updates: &HashMap<StateKey, Option<StateValue>>,
) -> Vec<(HashValue, Option<(HashValue, StateKey)>)> {
    state_updates
        .iter()
        .map(|(k, v_opt)| (k.hash(), v_opt.as_ref().map(|v| (v.hash(), k.clone()))))
        .collect()
}

pub fn jmt_update_refs<K>(
    jmt_updates: &[(HashValue, Option<(HashValue, K)>)],
) -> Vec<(HashValue, Option<&(HashValue, K)>)> {
    jmt_updates.iter().map(|(x, y)| (*x, y.as_ref())).collect()
}
