use std::fs;
use std::path::Path;
use aptos_config::config::{BUFFERED_STATE_TARGET_ITEMS, DEFAULT_MAX_NUM_NODES_PER_LRU_CACHE_SHARD, NO_OP_STORAGE_PRUNER_CONFIG, PrunerConfig, RocksdbConfigs};
use aptos_config::utils::get_genesis_txn;
use aptos_db::AptosDB;
use aptos_executor::db_bootstrapper::{generate_waypoint, maybe_bootstrap};
use aptos_jellyfish_merkle::metrics::{APTOS_JELLYFISH_INTERNAL_ENCODED_BYTES, APTOS_JELLYFISH_LEAF_ENCODED_BYTES};
use aptos_storage_interface::DbReaderWriter;
use aptos_types::transaction::Version;
use aptos_vm::AptosVM;
use crate::eth_transaction_generator::EthTransactionGenerator;
use crate::init_db_and_executor;
use crate::pipeline::Pipeline;
use crate::transaction_generator::TransactionGenerator;


pub fn run_eth_create_db(
    num_accounts: usize,
    init_account_balance: u64,
    block_size: usize,
    db_dir: impl AsRef<Path>,
    storage_pruner_config: PrunerConfig,
    verify_sequence_numbers: bool,
) {
    run(
        num_accounts,
        init_account_balance,
        block_size,
        db_dir,
        storage_pruner_config,
        verify_sequence_numbers,
    )
}

pub fn run(
    num_accounts: usize,
    init_account_balance: u64,
    block_size: usize,
    db_dir: impl AsRef<Path>,
    storage_pruner_config: PrunerConfig,
    verify_sequence_numbers: bool,
) {
    println!("Initializing...");

    if db_dir.as_ref().exists() {
        panic!("data-dir exists already.");
    }
    // create if not exists
    fs::create_dir_all(db_dir.as_ref()).unwrap();

    bootstrap_with_genesis(&db_dir);

    println!(
        "Finished empty DB creation, DB dir: {}. Creating accounts now...",
        db_dir.as_ref().display()
    );

    add_accounts_impl_eth(
        num_accounts,
        init_account_balance,
        block_size,
        &db_dir,
        &db_dir,
        storage_pruner_config,
        verify_sequence_numbers,
    );
}

fn bootstrap_with_genesis(db_dir: impl AsRef<Path>) {
    let (config, _genesis_key) = aptos_genesis::test_utils::test_config();
    // Create executor.
    let mut rocksdb_configs = RocksdbConfigs::default();
    rocksdb_configs.state_merkle_db_config.max_open_files = -1;
    let (_db, db_rw) = DbReaderWriter::wrap(
        AptosDB::open(
            &db_dir,
            false, /* readonly */
            NO_OP_STORAGE_PRUNER_CONFIG,
            rocksdb_configs,
            false, /* indexer */
            BUFFERED_STATE_TARGET_ITEMS,
            DEFAULT_MAX_NUM_NODES_PER_LRU_CACHE_SHARD,
        )
            .expect("DB should open."),
    );

    // Bootstrap db with genesis
    let waypoint = generate_waypoint::<AptosVM>(&db_rw, get_genesis_txn(&config).unwrap()).unwrap();
    maybe_bootstrap::<AptosVM>(&db_rw, get_genesis_txn(&config).unwrap(), waypoint).unwrap();
}


fn add_accounts_impl_eth(
    num_new_accounts: usize,
    init_account_balance: u64,
    block_size: usize,
    source_dir: impl AsRef<Path>,
    output_dir: impl AsRef<Path>,
    pruner_config: PrunerConfig,
    verify_sequence_numbers: bool,
) {
    let (mut config, genesis_key) = aptos_genesis::test_utils::test_config();
    config.storage.dir = output_dir.as_ref().to_path_buf();
    config.storage.storage_pruner_config = pruner_config;
    let (db, executor) = init_db_and_executor(&config);

    let version: Version = db.reader.get_latest_version().unwrap();

    let (pipeline, block_sender) = Pipeline::new(executor, version);

    let mut generator = EthTransactionGenerator::new_with_existing_db(
        db.clone(),
        genesis_key,
        block_sender,
        &source_dir,
        version,
    );

    generator.run_mint(
        db.reader.clone(),
        generator.num_existing_accounts(),
        num_new_accounts,
        init_account_balance,
        block_size,
    );
    generator.drop_sender();
    pipeline.join();

    if verify_sequence_numbers {
        println!("Verifying sequence numbers...");
        // Do a sanity check on the sequence number to make sure all transactions are committed.
        generator.verify_sequence_numbers(db.reader);
    }

    println!(
        "Created {} new accounts. Now at version {}, total # of accounts {}.",
        num_new_accounts,
        generator.version(),
        generator.num_existing_accounts() + num_new_accounts,
    );

    // Write metadata
    generator.write_meta(&output_dir, num_new_accounts);

    println!(
        "Total written internal nodes value size: {} bytes",
        APTOS_JELLYFISH_INTERNAL_ENCODED_BYTES.get()
    );
    println!(
        "Total written leaf nodes value size: {} bytes",
        APTOS_JELLYFISH_LEAF_ENCODED_BYTES.get()
    );
}