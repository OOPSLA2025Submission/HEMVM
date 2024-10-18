module aptos_framework::cross_vm {
    use std::string;
    use std::error;
    use std::signer;
    use std::option::Option;
    use aptos_std::type_info;
    use aptos_framework::coin::Coin;
    use aptos_framework::aptos_coin::AptosCoin;

    const ECOIN_INFO_ADDRESS_MISMATCH: u64 = 1;

    struct CallEvmCap<phantom CallType> has copy, store, drop {}

    struct DefaultCallType has copy, store {}

    public fun initialize_cap<CallType>(
        account: &signer,
    ): CallEvmCap<CallType> {
        let account_addr = signer::address_of(account);

        assert!(
            type_info::account_address(&type_info::type_of<CallType>()) == account_addr,
            error::invalid_argument(ECOIN_INFO_ADDRESS_MISMATCH),
        );

        CallEvmCap<CallType> {}
    }

    public fun default_cap(): CallEvmCap<DefaultCallType> {
        CallEvmCap<DefaultCallType> {}
    }

    public native fun call_evm<CallType>(coin: Option<Coin<AptosCoin>>, address: vector<u8>, function: string::String, params: vector<vector<u8>>, cap: &CallEvmCap<CallType>): vector<u8>;
}