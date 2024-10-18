use aptos_types::transaction::{
    aptos_address_to_eth_address, EthTransactionPayload, SignedTransaction, TransactionPayload,
};
use cfx_evm::TransactionInfo;
use cfx_primitives::Action;
use cfx_types::{AddressSpaceUtil, AddressWithSpace, U256};
use std::borrow::Cow;
use Cow::{Borrowed, Owned};

pub struct EvmTransaction<'a> {
    transaction: &'a SignedTransaction,
    payload: &'a EthTransactionPayload,
}

// Important (0xuki): deciding whether a tx is an ETH one or not in aptos_vm.rs
impl<'a> TryFrom<&'a SignedTransaction> for EvmTransaction<'a> {
    type Error = ();

    fn try_from(transaction: &'a SignedTransaction) -> Result<Self, Self::Error> {
        if let TransactionPayload::EthTransactionPayload(payload) = transaction.payload() {
            Ok(Self {
                transaction,
                payload,
            })
        } else {
            Err(())
        }
    }
}

impl<'a> TransactionInfo for EvmTransaction<'a> {
    fn sender(&self) -> Cow<AddressWithSpace> {
        let aptos_address = self.transaction.sender();
        let eth_address = aptos_address_to_eth_address(&aptos_address).with_evm_space();
        Owned(eth_address)
    }

    fn nonce(&self) -> Cow<U256> {
        Owned(U256::from(self.transaction.sequence_number()))
    }

    fn gas(&self) -> Cow<U256> {
        Owned(U256::from(self.transaction.max_gas_amount()))
    }

    fn gas_price(&self) -> Cow<U256> {
        Owned(U256::from(self.transaction.gas_unit_price()))
    }

    fn data(&self) -> Cow<[u8]> {
        Borrowed(&self.payload.data)
    }

    fn action(&self) -> Cow<Action> {
        Owned((&self.payload.action).into())
    }

    fn value(&self) -> Cow<U256> {
        Borrowed(&self.payload.value)
    }
}
