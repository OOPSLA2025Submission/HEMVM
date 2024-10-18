// Copyright 2019-2021 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

// Copyright 2015-2020 Parity Technologies (UK) Ltd.
// This file is part of OpenEthereum.

// OpenEthereum is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// OpenEthereum is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with OpenEthereum.  If not, see <http://www.gnu.org/licenses/>.

use aptos_api_types::HexEncodedBytes;
use cfx_primitives::log_entry::LogEntry;
use ethereum_types::{H160, H256, U256};
use serde::{Deserialize, Serialize};

/// Log
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Log {
    /// H160
    pub address: H160,
    /// Topics
    pub topics: Vec<H256>,
    /// Data
    pub data: HexEncodedBytes,
    /// Block Hash
    pub block_hash: H256,
    /// Block Number
    pub block_number: U256,
    /// Transaction Hash
    pub transaction_hash: H256,
    /// Transaction Index
    pub transaction_index: U256,
    /// Log Index in Block
    pub log_index: Option<U256>,
    /// Log Index in Transaction
    pub transaction_log_index: Option<U256>,
    /// Whether Log Type is Removed (Geth Compatibility Field)
    #[serde(default)]
    pub removed: bool,
}

impl Log {
    pub fn try_from(e: LogEntry) -> anyhow::Result<Log> {
        Ok(Log {
            address: e.address,
            topics: e.topics,
            data: e.data.into(),
            // FIXME(lpl): Set block/tx fields.
            block_hash: Default::default(),
            block_number: Default::default(),
            transaction_hash: Default::default(),
            transaction_index: Default::default(),
            log_index: Some(U256::zero()),
            transaction_log_index: Some(U256::zero()),
            removed: false,
        })
    }
}
