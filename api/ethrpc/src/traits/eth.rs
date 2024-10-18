use crate::types::{
    block::Block, block_number::BlockNumber, call_request::CallRequest, receipts::Receipt,
    transaction::Transaction,
};
use aptos_api_types::HexEncodedBytes;
use ethereum_types::{H160, H256, U256, U64};
use jsonrpsee::{core::RpcResult, proc_macros::rpc};

/// Eth rpc interface.
#[rpc(server, client, namespace = "eth")]
pub trait Eth {
    #[method(name = "version", aliases = ["net_version"])]
    async fn net_version(&self) -> RpcResult<String>;

    /// Sends signed transaction, returning its hash.
    #[method(name = "sendRawTransaction")]
    async fn send_raw_transaction(&self, bytes: HexEncodedBytes) -> RpcResult<H256>;

    /// Returns transaction receipt by transaction hash.
    #[method(name = "getTransactionReceipt")]
    async fn transaction_receipt(&self, tx_hash: H256) -> RpcResult<Option<Receipt>>;

    /// Returns balance of the given account.
    #[method(name = "getBalance")]
    async fn balance(&self, address: H160, block_number: Option<BlockNumber>) -> RpcResult<U256>;

    /// Call contract, returning the output data.
    #[method(name = "call")]
    async fn call(
        &self,
        request: CallRequest,
        maybe_block_number: Option<BlockNumber>,
    ) -> RpcResult<HexEncodedBytes>;

    /// Estimate gas needed for execution of given contract.
    #[method(name = "estimateGas")]
    async fn estimate_gas(
        &self,
        request: CallRequest,
        maybe_block_number: Option<BlockNumber>,
    ) -> RpcResult<U256>;

    /// Returns the chain ID used for transaction signing at the
    /// current best block. None is returned if not
    /// available.
    #[method(name = "chainId")]
    async fn chain_id(&self) -> RpcResult<Option<U64>>;

    /// Returns highest block number.
    #[method(name = "blockNumber")]
    async fn block_number(&self) -> RpcResult<U256>;

    /// Returns block with given number.
    #[method(name = "getBlockByNumber")]
    async fn block_by_number(
        &self,
        block_number: BlockNumber,
        include_txs: bool,
    ) -> RpcResult<Option<Block>>;

    /// Returns block with given hash.
    #[method(name = "getBlockByHash")]
    async fn block_by_hash(&self, block_hash: H256, include_txs: bool) -> RpcResult<Option<Block>>;

    /// Returns accounts list.
    #[method(name = "accounts")]
    async fn accounts(&self) -> RpcResult<Vec<H160>>;

    /// Returns the code at given address at given time (block number).
    #[method(name = "getCode")]
    async fn code_at(
        &self,
        address: H160,
        block_number: Option<BlockNumber>,
    ) -> RpcResult<HexEncodedBytes>;

    /// Returns the number of transactions sent from given address at given time
    /// (block number).
    #[method(name = "getTransactionCount")]
    async fn transaction_count(
        &self,
        address: H160,
        maybe_block_number: Option<BlockNumber>,
    ) -> RpcResult<U256>;

    /// Returns current gas_price.
    #[method(name = "gasPrice")]
    async fn gas_price(&self) -> RpcResult<U256>;

    /// Get transaction by its hash.
    #[method(name = "getTransactionByHash")]
    async fn transaction_by_hash(&self, h: H256) -> RpcResult<Option<Transaction>>;

    #[method(name = "maxPriorityFeePerGas")]
    async fn max_priority_fee(&self) -> RpcResult<U256>;

    #[method(name = "stressTestUniswap")]
    async fn stress_test_eth_txs_uniswap(&self) -> RpcResult<U256>;
}
