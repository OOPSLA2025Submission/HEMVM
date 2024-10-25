#![allow(unused_variables)]
// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

#[macro_use]
extern crate error_chain;
#[allow(unused)]
#[macro_use]
extern crate log;

mod error;
mod impls;
mod statedb_ext;

#[cfg(test)]
mod tests;

pub use self::{
    error::{Error, ErrorKind, Result},
    impls::StateDb,
    statedb_ext::{StateDbExt, TOTAL_TOKENS_KEY},
};
use cfx_internal_common::debug::ComputeEpochDebugRecord;
use primitives::{EpochId, StateKey};

pub trait StateDbTrait {
    fn get_raw(&self, key: StateKey) -> Result<Option<Box<[u8]>>>;

    fn set_raw(
        &mut self,
        key: StateKey,
        value: Box<[u8]>,
        debug_record: Option<&mut ComputeEpochDebugRecord>,
    ) -> Result<()>;

    fn delete(
        &mut self,
        key: StateKey,
        debug_record: Option<&mut ComputeEpochDebugRecord>,
    ) -> Result<()>;

    fn commit(
        &mut self,
        epoch_id: EpochId,
        debug_record: Option<&mut ComputeEpochDebugRecord>,
    ) -> Result<()>;
}
