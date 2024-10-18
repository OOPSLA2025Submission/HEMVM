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
use cfx_primitives::{Action, Eip155Transaction, SignedTransaction};
use cfx_types::AddressSpaceUtil;
use ethereum_types::{Address, H160, U256};
use serde::{Deserialize, Serialize};

// TODO(lpl): Set a proper value.
pub const MAX_GAS_CALL_REQUEST: u64 = 15_000_000;

/// Call request
#[derive(Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallRequest {
    /// From
    pub from: Option<H160>,
    /// To
    pub to: Option<H160>,
    /// Gas Price
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_price: Option<U256>,
    /// Max fee per gas
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fee_per_gas: Option<U256>,
    /// Gas
    pub gas: Option<U256>,
    /// Value
    pub value: Option<U256>,
    /// Data
    pub data: Option<HexEncodedBytes>,
    /// Nonce
    pub nonce: Option<U256>,
    /// Miner bribe
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_priority_fee_per_gas: Option<U256>,
}

pub fn sign_call(chain_id: u32, request: CallRequest) -> anyhow::Result<SignedTransaction> {
    let max_gas = U256::from(MAX_GAS_CALL_REQUEST);
    let gas = std::cmp::min(request.gas.unwrap_or(max_gas), max_gas);
    let from = request.from.unwrap_or_else(|| Address::zero());

    Ok(Eip155Transaction {
        nonce: request.nonce.unwrap_or_default(),
        action: request.to.map_or(Action::Create, |addr| Action::Call(addr)),
        gas,
        gas_price: request.gas_price.unwrap_or(1.into()),
        value: request.value.unwrap_or_default(),
        chain_id: Some(chain_id),
        data: request.data.unwrap_or(vec![].into()).0,
    }
    .fake_sign_rpc(from.with_evm_space()))
}

// impl Into<Request> for CallRequest {
//     fn into(self) -> Request {
//         Request {
//             transaction_type: self.transaction_type,
//             from: self.from.map(Into::into),
//             to: self.to.map(Into::into),
//             gas_price: self.gas_price.map(Into::into),
//             max_fee_per_gas: self.max_fee_per_gas,
//             gas: self.gas.map(Into::into),
//             value: self.value.map(Into::into),
//             data: self.data.map(Into::into),
//             nonce: self.nonce.map(Into::into),
//             access_list: self.access_list.map(Into::into),
//             max_priority_fee_per_gas:
// self.max_priority_fee_per_gas.map(Into::into),         }
//     }
// }
//
// #[cfg(test)]
// mod tests {
//     use super::CallRequest;
//     use ethereum_types::{H160, U256};
//     use rustc_hex::FromHex;
//     use serde_json;
//     use std::str::FromStr;
//
//     #[test]
//     fn call_request_deserialize() {
//         let s = r#"{
// 			"from":"0x0000000000000000000000000000000000000001",
// 			"to":"0x0000000000000000000000000000000000000002",
// 			"gasPrice":"0x1",
// 			"gas":"0x2",
// 			"value":"0x3",
// 			"data":"0x123456",
// 			"nonce":"0x4"
// 		}"#;
//         let deserialized: CallRequest = serde_json::from_str(s).unwrap();
//
//         assert_eq!(
//             deserialized,
//             CallRequest {
//                 transaction_type: Default::default(),
//                 from: Some(H160::from_low_u64_be(1)),
//                 to: Some(H160::from_low_u64_be(2)),
//                 gas_price: Some(U256::from(1)),
//                 max_fee_per_gas: None,
//                 gas: Some(U256::from(2)),
//                 value: Some(U256::from(3)),
//                 data: Some(vec![0x12, 0x34, 0x56].into()),
//                 nonce: Some(U256::from(4)),
//                 access_list: None,
//                 max_priority_fee_per_gas: None,
//             }
//         );
//     }
//
//     #[test]
//     fn call_request_deserialize2() {
//         let s = r#"{
// 			"from": "0xb60e8dd61c5d32be8058bb8eb970870f07233155",
// 			"to": "0xd46e8dd67c5d32be8058bb8eb970870f07244567",
// 			"gas": "0x76c0",
// 			"gasPrice": "0x9184e72a000",
// 			"value": "0x9184e72a",
// 			"data":
// "0xd46e8dd67c5d32be8d46e8dd67c5d32be8058bb8eb970870f072445675058bb8eb970870f072445675"
// 		}"#;
//         let deserialized: CallRequest = serde_json::from_str(s).unwrap();
//
//         assert_eq!(deserialized, CallRequest {
//             transaction_type: Default::default(),
// 			from: Some(H160::from_str("b60e8dd61c5d32be8058bb8eb970870f07233155").
// unwrap()), 			to: Some(H160::from_str("d46e8dd67c5d32be8058bb8eb970870f07244567"
// ).unwrap()), 			gas_price: Some(U256::from_str("9184e72a000").unwrap()),
// 			max_fee_per_gas: None,
// 			gas: Some(U256::from_str("76c0").unwrap()),
// 			value: Some(U256::from_str("9184e72a").unwrap()),
// 			data: Some("d46e8dd67c5d32be8d46e8dd67c5d32be8058bb8eb970870f072445675058bb8eb970870f072445675".from_hex().unwrap().into()),
// 			nonce: None,
// 			access_list: None,
// 			max_priority_fee_per_gas: None,
// 		});
//     }
//
//     #[test]
//     fn call_request_deserialize_empty() {
//         let s = r#"{"from":"0x0000000000000000000000000000000000000001"}"#;
//         let deserialized: CallRequest = serde_json::from_str(s).unwrap();
//
//         assert_eq!(
//             deserialized,
//             CallRequest {
//                 transaction_type: Default::default(),
//                 from: Some(H160::from_low_u64_be(1)),
//                 to: None,
//                 gas_price: None,
//                 max_fee_per_gas: None,
//                 gas: None,
//                 value: None,
//                 data: None,
//                 nonce: None,
//                 access_list: None,
//                 max_priority_fee_per_gas: None,
//             }
//         );
//     }
// }
