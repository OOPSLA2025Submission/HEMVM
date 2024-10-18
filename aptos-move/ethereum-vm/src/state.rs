pub use crate::storage_key::StorageKey;
use aptos_state_view::StateView;
use aptos_types::{state_store::state_key::StateKey, write_set::WriteOp};
use cfx_evm::{CallMoveVMTrait, State, StateDb, StorageTrait};
use cfx_storage::Result;
use cfx_types::H256;
use std::collections::HashMap;

pub struct ViewWrapper<'a, S: StateView> {
    pub inner: &'a S,
    pub cache: HashMap<StorageKey, Option<Box<[u8]>>>,
}

impl<'a, S: StateView> StorageTrait for &mut ViewWrapper<'a, S> {
    type StorageKey = StorageKey;
    // IMPORTANT: ViewWrapper cache hit-or-miss
    fn get(&self, key: StorageKey) -> Result<Option<Box<[u8]>>> {
        match self.cache.get(&key) {
            Some(cached_value) => {
                return Ok(cached_value.clone());
            },
            None => {},
        };
        Ok(self
            .inner
            .get_state_value(&key.into())
            .unwrap()
            .map(|value| value.into_boxed_slice()))
    }

    fn set(&mut self, access_key: StorageKey, value: Box<[u8]>) -> Result<()> {
        self.cache.insert(access_key, Some(value));
        Ok(())
    }

    fn delete(&mut self, access_key: StorageKey) -> Result<()> {
        self.cache.insert(access_key, None);
        Ok(())
    }

    fn commit(&mut self, _epoch: H256) -> Result<()> {
        Ok(())
    }
}

impl<'a, S: StateView> ViewWrapper<'a, S> {
    pub fn drain(mut self) -> Vec<(StateKey, WriteOp)> {
        self.cache
            .drain()
            .filter_map(|(key, value)| {
                let state_key: StateKey = key.into();
                let has_old_value = self
                    .inner
                    .get_state_value(&state_key)
                    .unwrap_or(None)
                    .is_some();
                match (has_old_value, value) {
                    (true, Some(v)) => Some(WriteOp::Modification(v.to_vec())),
                    (true, None) => Some(WriteOp::Deletion),
                    (false, Some(v)) => Some(WriteOp::Creation(v.to_vec())),
                    (false, None) => None,
                }
                .map(|op| (state_key, op))
            })
            .collect()
    }
}

pub struct EvmState<'a> {
    pub state: State<'a>,
}

impl<'a> EvmState<'a> {
    pub fn new<S: StateView>(view_wrapper: &'a mut ViewWrapper<S>) -> Self {
        let state_db = StateDb::new(view_wrapper); // wrapper that is located on the heap
        let state = State::new(state_db).unwrap();
        Self { state }
    }

    pub fn new_with_move_vm<S: StateView>(
        view_wrapper: &'a mut ViewWrapper<S>,
        call_move_handler: &'a mut dyn CallMoveVMTrait,
    ) -> Self {
        let state_db = StateDb::new(view_wrapper);
        let state = State::new_with_move_vm(state_db, call_move_handler).unwrap();
        Self { state }
    }
}
