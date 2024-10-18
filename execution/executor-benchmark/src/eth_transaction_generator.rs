use std::convert::TryInto;
use std::fs::File;
use std::io::{Read, Write};
use std::iter::once;
use std::path::Path;
use rayon::iter::ParallelIterator;
use std::sync::{Arc, mpsc};
use aptos_crypto::ed25519::Ed25519PrivateKey;
use aptos_sdk::transaction_builder::{EthTransactionFactory, TransactionFactory};
use aptos_sdk::types::LocalAccount;
use aptos_storage_interface::{DbReader, DbReaderWriter};
use aptos_types::account_config::aptos_test_root_address;
use aptos_types::chain_id::ChainId;
use crate::account_generator::{AccountCache, AccountGenerator};
use crate::transaction_generator::{get_progress_bar, get_sequence_number_aptos, P2pTestCase, TestCase};
use chrono::Local;
use itertools::Itertools;
use rayon::iter::IntoParallelRefIterator;
use aptos_crypto::HashValue;
use aptos_state_view::account_with_state_view::AsAccountWithStateView;
use aptos_storage_interface::state_view::LatestDbStateCheckpointView;
use aptos_types::account_view::AccountView;
use aptos_types::transaction::{eth_address_to_aptos_address, EthAddress, SignedTransaction, Transaction, Version};
use aptos_types::transaction::Transaction::UserTransaction;
use cfx_types::U256;
use crate::eth_account_generator::{EthAccountCache, EthAccountGenerator};

const META_FILENAME: &str = "metadata.toml";
const MAX_ACCOUNTS_INVOLVED_IN_P2P: usize = 1_000_000;

// will be changed in the future; for now, one address receives all the money in the bank, and then
// redistributes it to the randomly generated addresses using transfers
const ETH_GENESIS_ADDRESS: &str = "redacted";

macro_rules! now_fmt {
    () => {
        Local::now().format("%m-%d %H:%M:%S")
    };
}


fn get_sequence_number_eth(address: EthAddress, reader: Arc<dyn DbReader>) -> u64 {
    let db_state_view = reader.latest_state_checkpoint_view().unwrap();

    let aptos_address = eth_address_to_aptos_address(&address);
    let account_state_view = db_state_view.as_account_with_state_view(&aptos_address);

    match account_state_view.get_account_resource().unwrap() {
        Some(account_resource) => account_resource.sequence_number(),
        None => 0,
    }
}

pub struct EthTransactionGenerator {
    /// The current state of the accounts. The main purpose is to keep track of the sequence number
    /// so generated transactions are guaranteed to be successfully executed.
    accounts_cache: Option<EthAccountCache>,

    /// The current state of seed accounts. The purpose of the seed accounts to parallelize the
    /// account creation and minting process so that they are not blocked on sequence number of
    /// a single root account.
    seed_accounts_cache: Option<EthAccountCache>,

    /// Total # of existing (non-seed) accounts in the DB at the time of TransactionGenerator
    /// creation.
    num_existing_accounts: usize,

    /// Record the number of txns generated.
    version: Version,

    /// Each generated block of transactions are sent to this channel. Using `SyncSender` to make
    /// sure if execution is slow to consume the transactions, we do not run out of memory.
    block_sender: Option<mpsc::SyncSender<Vec<Transaction>>>,

    /// Transaction Factory
    transaction_factory: EthTransactionFactory,

    /// root account is used across creating and minting.
    root_account: LocalAccount,
}


impl EthTransactionGenerator {
    pub fn new_with_existing_db<P: AsRef<Path>>(
        db: DbReaderWriter,
        genesis_key: Ed25519PrivateKey,
        block_sender: mpsc::SyncSender<Vec<Transaction>>,
        db_dir: P,
        version: Version,
    ) -> Self {
        let path = db_dir.as_ref().join(META_FILENAME);

        let num_existing_accounts = File::open(path).map_or(0, |mut file| {
            let mut contents = vec![];
            file.read_to_end(&mut contents).unwrap();
            let test_case: TestCase = toml::from_slice(&contents).expect("Must exist.");
            let TestCase::P2p(P2pTestCase { num_accounts }) = test_case;
            num_accounts
        });

        let num_cached_accounts =
            std::cmp::min(num_existing_accounts, MAX_ACCOUNTS_INVOLVED_IN_P2P);
        let accounts_cache = Some(Self::gen_user_account_cache(num_cached_accounts));

        Self {
            seed_accounts_cache: None,
            root_account: LocalAccount::new(
                aptos_test_root_address(),
                genesis_key,
                get_sequence_number_aptos(aptos_test_root_address(), db.reader),
            ),
            accounts_cache,
            num_existing_accounts,
            version,
            block_sender: Some(block_sender),
            transaction_factory: Self::create_transaction_factory(),
        }
    }

    fn create_transaction_factory() -> EthTransactionFactory {
        EthTransactionFactory::new(ChainId::test())
            .with_transaction_expiration_time(300)
            .with_gas_unit_price(100)
            // TODO(Gas): double check if this is correct
            .with_max_gas_amount(100_000)
    }

    fn gen_user_account_cache(num_accounts: usize) -> EthAccountCache {
        Self::gen_account_cache(
            EthAccountGenerator::new_for_user_accounts(0),
            num_accounts,
            "user",
        )
    }

    fn gen_account_cache(
        generator: EthAccountGenerator,
        num_accounts: usize,
        name: &str,
    ) -> EthAccountCache {
        println!(
            "[{}] Generating cache of {} {} accounts.",
            now_fmt!(),
            num_accounts,
            name,
        );
        let mut accounts = EthAccountCache::new(generator);
        let bar = get_progress_bar(num_accounts);
        for _ in 0..num_accounts {
            accounts.grow(1);
            bar.inc(1);
        }
        bar.finish();
        accounts
    }

    pub fn num_existing_accounts(&self) -> usize {
        self.num_existing_accounts
    }

    pub fn run_mint(
        &mut self,
        reader: Arc<dyn DbReader>,
        num_existing_accounts: usize,
        num_new_accounts: usize,
        init_account_balance: u64,
        block_size: usize,
    ) {
        assert!(self.block_sender.is_some());
        // Ensure that seed accounts have enough balance to transfer money to at least 1000 account with
        // balance init_account_balance.
        self.create_seed_accounts(
            reader,
            num_new_accounts,
            block_size,
            init_account_balance * 1_000_000_000,
        );
        // self.create_and_fund_accounts(
        //     num_existing_accounts,
        //     num_new_accounts,
        //     init_account_balance,
        //     block_size,
        // );
    }

    pub fn create_seed_accounts(
        &mut self,
        reader: Arc<dyn DbReader>,
        num_new_accounts: usize,
        block_size: usize,
        seed_account_balance: u64,
    ) -> Vec<Vec<Transaction>> {
        let mut txn_block = Vec::new();

        // We don't store the # of existing seed accounts now. Thus here we just blindly re-create
        // and re-mint seed accounts here.

        // let num_seed_accounts = (num_new_accounts / 1000).clamp(1, 100000);
        // let num_seed_accounts = num_new_accounts;
        // let seed_accounts_cache = Self::gen_seed_account_cache(reader, num_seed_accounts);

        // println!(
        //     "[{}] Generating {} seed account creation txns.",
        //     now_fmt!(),
        //     num_seed_accounts,
        // );
        // let bar = get_progress_bar(num_seed_accounts);

        // for chunk in seed_accounts_cache
        //     .accounts
        //     .iter()
        //     .collect::<Vec<_>>()
        //     .chunks(block_size)
        // {
        //     let transactions = chunk
        //         .iter()
        //         .flat_map(|account| {
        //             let mint = self.transaction_factory.mint(
        //                 account.address(),
        //                 seed_account_balance,
        //                 U256::from(0),
        //                 4
        //             );
        //             vec![mint]
        //         })
        //         .map(SignedTransaction::try_from);

        //     let mut converted_aptos_txs: Vec<_> = Vec::new();

        //     for t in transactions {
        //         converted_aptos_txs.push(UserTransaction(t.expect("")));
        //     }
        //     converted_aptos_txs = converted_aptos_txs
        //         .into_iter()
        //         .chain(once(Transaction::StateCheckpoint(HashValue::random())))
        //         .collect();

        //     self.version += converted_aptos_txs.len() as Version;
        //     bar.inc(converted_aptos_txs.len() as u64 - 1);
        //     if let Some(sender) = &self.block_sender {
        //         sender.send(converted_aptos_txs).unwrap();
        //     } else {
        //         txn_block.push(converted_aptos_txs);
        //     }
        // }
        // bar.finish();
        // println!("[{}] done.", now_fmt!());
        // self.seed_accounts_cache = Some(seed_accounts_cache);

        txn_block
    }

    fn gen_seed_account_cache(reader: Arc<dyn DbReader>, num_accounts: usize) -> EthAccountCache {
        let generator = EthAccountGenerator::new_for_seed_accounts();

        let mut accounts = Self::gen_account_cache(generator, num_accounts, "seed");

        for account in &mut accounts.accounts {
            *account.sequence_number_mut() = get_sequence_number_eth(account.address(), reader.clone());
        }
        accounts
    }

    pub fn create_and_fund_accounts(
        &mut self,
        num_existing_accounts: usize,
        num_new_accounts: usize,
        init_account_balance: u64,
        block_size: usize,
    ) -> Vec<Vec<Transaction>> {
        let mut txn_block = vec![];

        println!(
            "[{}] Generating {} account creation txns.",
            now_fmt!(),
            num_new_accounts
        );
        let mut generator = AccountGenerator::new_for_user_accounts(num_existing_accounts as u64);
        println!("Skipped first {} existing accounts.", num_existing_accounts);

        let bar = get_progress_bar(num_new_accounts);

        // for chunk in &(0..num_new_accounts).chunks(block_size) {
        //     let transactions: Vec<_> = chunk
        //         .map(|_| {
        //             self.seed_accounts_cache
        //                 .as_mut()
        //                 .unwrap()
        //                 .get_random()
        //                 .sign_with_transaction_builder(
        //                     self.transaction_factory
        //                         .implicitly_create_user_account_and_transfer(
        //                             generator.generate().public_key(),
        //                             init_account_balance,
        //                         ),
        //                 )
        //         })
        //         .map(Transaction::UserTransaction)
        //         .chain(once(Transaction::StateCheckpoint(HashValue::random())))
        //         .collect();
        //     self.version += transactions.len() as Version;
        //     if let Some(sender) = &self.block_sender {
        //         sender.send(transactions).unwrap();
        //     } else {
        //         txn_block.push(transactions);
        //     }
        //     bar.inc(block_size as u64);
        // }
        bar.finish();
        println!("[{}] done.", now_fmt!());

        txn_block
    }

    /// Drops the sender to notify the receiving end of the channel.
    pub fn drop_sender(&mut self) {
        self.block_sender.take().unwrap();
    }

    pub fn verify_sequence_numbers(&self, db: Arc<dyn DbReader>) {
        if self.accounts_cache.is_none() {
            println!("Cannot verify account sequence numbers.");
            return;
        }

        let num_accounts_in_cache = self.accounts_cache.as_ref().unwrap().len();
        println!(
            "[{}] verify {} account sequence numbers.",
            now_fmt!(),
            num_accounts_in_cache,
        );
        let bar = get_progress_bar(num_accounts_in_cache);
        self.accounts_cache
            .as_ref()
            .unwrap()
            .accounts()
            .par_iter()
            .for_each(|account| {
                let address = account.address();
                let db_state_view = db.latest_state_checkpoint_view().unwrap();
                let aptos_address = eth_address_to_aptos_address(&address);
                let address_account_view = db_state_view.as_account_with_state_view(&aptos_address);
                assert_eq!(
                    address_account_view
                        .get_account_resource()
                        .unwrap()
                        .unwrap()
                        .sequence_number(),
                    account.sequence_number()
                );
                bar.inc(1);
            });
        bar.finish();
        println!("[{}] done.", now_fmt!());
    }

    pub fn version(&self) -> Version {
        self.version
    }

    pub fn write_meta<P: AsRef<Path>>(self, path: &P, num_new_accounts: usize) {
        let metadata = TestCase::P2p(P2pTestCase {
            num_accounts: self.num_existing_accounts + num_new_accounts,
        });
        let serialized = toml::to_vec(&metadata).unwrap();
        let meta_file = path.as_ref().join(META_FILENAME);
        let mut file = File::create(meta_file).unwrap();
        file.write_all(&serialized).unwrap();
    }

}