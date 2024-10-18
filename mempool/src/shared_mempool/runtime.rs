// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

use crate::{
    core_mempool::CoreMempool,
    network::{MempoolNetworkEvents, MempoolSyncMsg},
    shared_mempool::{
        coordinator::{coordinator, gc_coordinator, snapshot_job},
        types::{MempoolEventsReceiver, SharedMempool, SharedMempoolNotification},
    },
    QuorumStoreRequest,
};
use aptos_config::{config::NodeConfig, network_id::NetworkId};
use aptos_event_notifications::ReconfigNotificationListener;
use aptos_infallible::{Mutex, RwLock};
use aptos_logger::Level;
use aptos_mempool_notifications::MempoolNotificationListener;
use aptos_network::{
    application::{interface::NetworkClient, storage::PeerMetadataStorage},
    protocols::{network::NetworkSender, wire::handshake::v1::ProtocolId::MempoolDirectSend},
};
use aptos_block_executor::state_view::DbReader;
use aptos_vm_validator::vm_validator::{TransactionValidation, VMValidator};
use futures::channel::mpsc::{self, Receiver, UnboundedSender};
use std::{collections::HashMap, sync::Arc};
use tokio::runtime::{Handle, Runtime};

/// Bootstrap of SharedMempool.
/// Creates a separate Tokio Runtime that runs the following routines:
///   - outbound_sync_task (task that periodically broadcasts transactions to peers).
///   - inbound_network_task (task that handles inbound mempool messages and network events).
///   - gc_task (task that performs GC of all expired transactions by SystemTTL).
pub(crate) fn start_shared_mempool<TransactionValidator>(
    executor: &Handle,
    config: &NodeConfig,
    mempool: Arc<Mutex<CoreMempool>>,
    // First element in tuple is the network ID.
    // See `NodeConfig::is_upstream_peer` for the definition of network ID.
    mempool_network_handles: Vec<(
        NetworkId,
        NetworkSender<MempoolSyncMsg>,
        MempoolNetworkEvents,
    )>,
    client_events: MempoolEventsReceiver,
    quorum_store_requests: mpsc::Receiver<QuorumStoreRequest>,
    mempool_listener: MempoolNotificationListener,
    mempool_reconfig_events: ReconfigNotificationListener,
    db: Arc<dyn DbReader>,
    validator: Arc<RwLock<TransactionValidator>>,
    subscribers: Vec<UnboundedSender<SharedMempoolNotification>>,
    peer_metadata_storage: Arc<PeerMetadataStorage>,
) where
    TransactionValidator: TransactionValidation + 'static,
{
    let mut all_network_events = vec![];
    let mut network_senders = HashMap::new();
    for (network_id, network_sender, network_events) in mempool_network_handles.into_iter() {
        all_network_events.push((network_id, network_events));
        network_senders.insert(network_id, network_sender);
    }

    let network_client = NetworkClient::new(
        vec![MempoolDirectSend],
        vec![],
        network_senders,
        peer_metadata_storage,
    );
    let smp: SharedMempool<NetworkClient<MempoolSyncMsg>, TransactionValidator> =
        SharedMempool::new(
            mempool.clone(),
            config.mempool.clone(),
            network_client,
            db,
            validator,
            subscribers,
            config.base.role,
        );

    executor.spawn(coordinator(
        smp,
        executor.clone(),
        all_network_events,
        client_events,
        quorum_store_requests,
        mempool_listener,
        mempool_reconfig_events,
    ));

    executor.spawn(gc_coordinator(
        mempool.clone(),
        config.mempool.system_transaction_gc_interval_ms,
    ));

    if aptos_logger::enabled!(Level::Trace) {
        executor.spawn(snapshot_job(
            mempool,
            config.mempool.mempool_snapshot_interval_secs,
        ));
    }
}

pub fn bootstrap(
    config: &NodeConfig,
    db: Arc<dyn DbReader>,
    // The first element in the tuple is the ID of the network that this network is a handle to.
    // See `NodeConfig::is_upstream_peer` for the definition of network ID.
    mempool_network_handles: Vec<(
        NetworkId,
        NetworkSender<MempoolSyncMsg>,
        MempoolNetworkEvents,
    )>,
    client_events: MempoolEventsReceiver,
    quorum_store_requests: Receiver<QuorumStoreRequest>,
    mempool_listener: MempoolNotificationListener,
    mempool_reconfig_events: ReconfigNotificationListener,
    peer_metadata_storage: Arc<PeerMetadataStorage>,
) -> Runtime {
    let runtime = aptos_runtimes::spawn_named_runtime("shared-mem".into(), None);
    let mempool = Arc::new(Mutex::new(CoreMempool::new(config)));
    let vm_validator = Arc::new(RwLock::new(VMValidator::new(Arc::clone(&db))));
    start_shared_mempool(
        runtime.handle(),
        config,
        mempool,
        mempool_network_handles,
        client_events,
        quorum_store_requests,
        mempool_listener,
        mempool_reconfig_events,
        db,
        vm_validator,
        vec![],
        peer_metadata_storage,
    );
    runtime
}