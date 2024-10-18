use crate::state::StorageKey;
use anyhow;
use aptos_types::{
    contract_event::ContractEvent,
    event::EventKey,
    transaction::{aptos_address_to_eth_address, eth_address_to_aptos_address},
};
use cfx_evm::StorageTrait;
use cfx_primitives::LogEntry;
use cfx_storage::Result as DbResult;
use cfx_types::{Address, Space, H256};
use move_core_types::language_storage::TypeTag;
use solidity_abi::{ABIDecodable, ABIEncodable};
use solidity_abi_derive::ABIVariable;
use std::collections::BTreeMap;

pub fn evm_events_to_aptos_events(
    storage: impl StorageTrait<StorageKey = StorageKey>,
    events: Vec<LogEntry>,
) -> DbResult<Vec<ContractEvent>> {
    Ok(assign_event_nonce(storage, events)?
        .into_iter()
        .map(Into::into)
        .collect())
}

pub fn aptos_events_to_evm_events(events: Vec<ContractEvent>) -> anyhow::Result<Vec<LogEntry>> {
    events
        .into_iter()
        .map(|e| LogEntryWithNonce::try_from(e).map(|l| l.log))
        .collect()
}

struct LogEntryWithNonce {
    log: LogEntry,
    nonce: u64,
}

#[derive(ABIVariable)]
struct LogEntryPayload {
    pub topics: Vec<H256>,
    /// The data associated with the `LOG` operation.
    pub data: Vec<u8>,
}

impl From<LogEntryWithNonce> for ContractEvent {
    fn from(value: LogEntryWithNonce) -> Self {
        use TypeTag::{Vector, U8};
        // TODO: currently, all the events from the same evm contract shares one event key. Maybe better solution later.
        let LogEntryWithNonce {
            log:
                LogEntry {
                    address,
                    topics,
                    data,
                    ..
                },
            nonce,
        } = value;
        let account_address = eth_address_to_aptos_address(&address);
        let event_key = EventKey::new(0, account_address);
        let payload = LogEntryPayload { topics, data };
        Self::new(event_key, nonce, Vector(Box::new(U8)), payload.abi_encode())
    }
}

impl TryFrom<ContractEvent> for LogEntryWithNonce {
    type Error = anyhow::Error;

    fn try_from(value: ContractEvent) -> Result<Self, Self::Error> {
        let LogEntryPayload { topics, data } =
            ABIDecodable::abi_decode(value.event_data()).map_err(|e| anyhow::anyhow!(e.0))?;
        let address = aptos_address_to_eth_address(&value.key().get_creator_address());
        let log = LogEntry {
            address,
            topics,
            data,
            space: Space::Ethereum,
        };
        Ok(Self {
            log,
            nonce: value.sequence_number(),
        })
    }
}

fn assign_event_nonce(
    mut storage: impl StorageTrait<StorageKey = StorageKey>,
    logs: Vec<LogEntry>,
) -> DbResult<Vec<LogEntryWithNonce>> {
    let mut nonce_map: BTreeMap<Address, u64> = BTreeMap::new();
    let answer: Vec<LogEntryWithNonce> = logs
        .into_iter()
        .map(|log| -> DbResult<LogEntryWithNonce> {
            let address = log.address;
            let nonce;
            if !nonce_map.contains_key(&address) {
                let current_nonce = read_nonce(&storage, address)?;
                nonce_map.insert(address, current_nonce + 1);
                nonce = current_nonce;
            } else {
                let nonce_mut = nonce_map.get_mut(&address).unwrap();
                nonce = *nonce_mut;
                *nonce_mut += 1;
            }

            Ok(LogEntryWithNonce { log, nonce })
        })
        .collect::<DbResult<_>>()?;

    for (address, nonce) in nonce_map {
        write_nonce(&mut storage, address, nonce)?;
    }

    Ok(answer)
}

fn read_nonce(
    storage: &impl StorageTrait<StorageKey = StorageKey>,
    address: Address,
) -> DbResult<u64> {
    let nonce = storage
        .get(StorageKey::event_nonce_key(address))?
        .map_or(0, |x| {
            let mut encoded = [0u8; 8];
            encoded.copy_from_slice(&x);
            u64::from_le_bytes(encoded)
        });
    Ok(nonce)
}
fn write_nonce(
    storage: &mut impl StorageTrait<StorageKey = StorageKey>,
    address: Address,
    nonce: u64,
) -> DbResult<()> {
    let encoded: Box<[u8]> = nonce.to_be_bytes().to_vec().into_boxed_slice();
    storage.set(StorageKey::event_nonce_key(address), encoded)
}
