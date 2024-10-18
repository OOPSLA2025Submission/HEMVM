// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

//! This module defines physical storage schema mapping block hash to the version the
//! `BlockMetadata` event.
//! With the version one can resort to `EventSchema` for the block content.
//!
//! ```text
//! |<--key-->|<-value->|
//! |   hash  | block_ver |
//! ```

use crate::schema::{ensure_slice_len_eq, BLOCK_VERSION_BY_HASH_CF_NAME};
use anyhow::Result;
use aptos_crypto::HashValue;
use aptos_schemadb::{
    define_schema,
    schema::{KeyCodec, ValueCodec},
};
use aptos_types::transaction::Version;
use byteorder::{BigEndian, ReadBytesExt};
use std::mem::size_of;

define_schema!(
    BlockVersionByHashSchema,
    HashValue,
    Version,
    BLOCK_VERSION_BY_HASH_CF_NAME
);

impl KeyCodec<BlockVersionByHashSchema> for HashValue {
    fn encode_key(&self) -> Result<Vec<u8>> {
        Ok(self.to_vec())
    }

    fn decode_key(data: &[u8]) -> Result<Self> {
        ensure_slice_len_eq(data, size_of::<Self>())?;
        Ok(HashValue::from_slice(data)?)
    }
}

impl ValueCodec<BlockVersionByHashSchema> for Version {
    fn encode_value(&self) -> Result<Vec<u8>> {
        Ok(self.to_be_bytes().to_vec())
    }

    fn decode_value(mut data: &[u8]) -> Result<Self> {
        ensure_slice_len_eq(data, size_of::<Self>())?;

        Ok(data.read_u64::<BigEndian>()?)
    }
}
