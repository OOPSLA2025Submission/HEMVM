// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

use crate::utils;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EthApiConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub address: SocketAddr,

    #[serde(default = "default_disabled")]
    pub keep_alive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cors: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threads: Option<usize>,
}

pub const DEFAULT_ADDRESS_ETH: &str = "0.0.0.0";
pub const DEFAULT_PORT_ETH: u16 = 8545;

fn default_enabled() -> bool {
    true
}

fn default_disabled() -> bool {
    false
}

impl Default for EthApiConfig {
    fn default() -> EthApiConfig {
        EthApiConfig {
            enabled: default_enabled(),
            address: format!("{}:{}", DEFAULT_ADDRESS_ETH, DEFAULT_PORT_ETH)
                .parse()
                .unwrap(),
            keep_alive: false,
            cors: None,
            threads: None,
        }
    }
}

impl EthApiConfig {
    pub fn randomize_ports(&mut self) {
        self.address.set_port(utils::get_available_port());
    }
}
