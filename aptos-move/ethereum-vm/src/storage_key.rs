use aptos_types::{
    access_path::{AccessPath, EvmPath, Path},
    state_store::state_key::StateKey as AptosStateKey,
    transaction::eth_address_to_aptos_address,
};
use cfx_primitives::OwnedStateKey;
use cfx_types::Address;
#[derive(PartialEq, Eq, Hash)]
pub struct StorageKey {
    address: Address,
    path: EvmPath,
}

impl StorageKey {
    pub fn event_nonce_key(address: Address) -> StorageKey {
        StorageKey {
            address,
            path: EvmPath::EventNonce,
        }
    }
}

impl From<OwnedStateKey> for StorageKey {
    fn from(key: OwnedStateKey) -> Self {
        let (address, path) = match key {
            OwnedStateKey::AccountKey(address) => (address, EvmPath::Account),
            OwnedStateKey::StorageKey {
                address,
                storage_key,
            } => (address, EvmPath::Storage(storage_key)),
            OwnedStateKey::CodeKey(address) => (address, EvmPath::Code),
        };
        Self {
            address: address.address,
            path,
        }
    }
}

// IMPORTANT: KEY CONVERSION FROM ETH TO MOVE
impl From<StorageKey> for AccessPath {
    fn from(key: StorageKey) -> Self {
        let address = eth_address_to_aptos_address(&key.address);
        let path = bcs::to_bytes(&Path::Evm(key.path)).expect("Unexpected Serialization Error");

        // const STORAGE_PREFIX: [u8; 5] = *b"store";
        // const CODE: [u8; 4] = *b"code";
        // const ACCOUNT: [u8; 7] = *b"account";
        // const EVENT_NONCE: [u8; 6] = *b"enonce";

        // let path = match key.path {
        //     EvmPath::Account => ACCOUNT.to_vec(),
        //     EvmPath::Storage(storage_path) => [&STORAGE_PREFIX[..], &storage_path].concat(),
        //     EvmPath::Code => CODE.to_vec(),
        //     EvmPath::EventNonce => EVENT_NONCE.to_vec(),
        // };

        Self { address, path }
    }
}

impl From<StorageKey> for AptosStateKey {
    fn from(key: StorageKey) -> Self {
        AptosStateKey::AccessPath(key.into())
    }
}
