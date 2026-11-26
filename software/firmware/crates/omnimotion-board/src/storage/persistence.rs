extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write as _;

use heapless::String;
use omnimotion::{
    encode_record, encoded_len, prepare_write, select_latest, EncodeError, Lifecycle, LoadError,
    RecordKind, RecordMeta, Slot, WriteRefusal,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::motion::Motion;
use crate::storage::{StorageError, Store, MAX_NAME_LEN};

#[derive(Debug)]
pub enum PersistenceError {
    Storage(StorageError),
    Record(LoadError),
    Encode(EncodeError),
    WriteRefused(WriteRefusal),
    PayloadEncode,
    PayloadDecode,
}

impl From<StorageError> for PersistenceError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<LoadError> for PersistenceError {
    fn from(error: LoadError) -> Self {
        Self::Record(error)
    }
}

impl From<EncodeError> for PersistenceError {
    fn from(error: EncodeError) -> Self {
        Self::Encode(error)
    }
}

#[derive(Debug)]
pub struct StoredRecord<T> {
    pub meta: RecordMeta,
    pub value: T,
}

#[derive(Clone, Copy, Debug)]
pub struct RecordSpec<'a> {
    pub namespace: &'a str,
    pub base_key: &'a str,
    pub kind: RecordKind,
    pub schema_version: u16,
}

/// Versioned, two-slot records over the board's NVS store.
pub struct TransactionalStore {
    store: Store,
}

impl TransactionalStore {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    pub fn load_bytes(
        &mut self,
        spec: RecordSpec<'_>,
    ) -> Result<StoredRecord<Vec<u8>>, PersistenceError> {
        let (key_a, key_b) = slot_keys(spec.base_key)?;
        let slot_a = self.store.read_blob(spec.namespace, &key_a)?;
        let slot_b = self.store.read_blob(spec.namespace, &key_b)?;
        let selected = select_latest(
            slot_a.as_deref(),
            slot_b.as_deref(),
            spec.kind,
            spec.schema_version,
        )?;
        Ok(StoredRecord {
            meta: selected.record.meta,
            value: selected.record.payload.to_vec(),
        })
    }

    pub fn save_bytes(
        &mut self,
        motion: &Motion,
        lifecycle: &Lifecycle,
        spec: RecordSpec<'_>,
        payload: &[u8],
    ) -> Result<RecordMeta, PersistenceError> {
        motion
            .persistence_gate(lifecycle)
            .check()
            .map_err(PersistenceError::WriteRefused)?;
        let (key_a, key_b) = slot_keys(spec.base_key)?;
        let slot_a = self.store.read_blob(spec.namespace, &key_a)?;
        let slot_b = self.store.read_blob(spec.namespace, &key_b)?;
        let pending = prepare_write(
            slot_a.as_deref(),
            slot_b.as_deref(),
            spec.kind,
            spec.schema_version,
        )?;
        let meta = RecordMeta {
            kind: spec.kind,
            schema_version: spec.schema_version,
            generation: pending.generation,
        };
        let len = encoded_len(payload.len()).ok_or(EncodeError::PayloadTooLarge)?;
        let mut bytes = vec![0; len];
        let encoded = encode_record(meta, payload, &mut bytes)?;
        let key = match pending.slot {
            Slot::A => &key_a,
            Slot::B => &key_b,
        };
        self.store.write_blob(spec.namespace, key, encoded)?;
        Ok(meta)
    }

    pub fn load<T: DeserializeOwned>(
        &mut self,
        spec: RecordSpec<'_>,
    ) -> Result<StoredRecord<T>, PersistenceError> {
        let stored = self.load_bytes(spec)?;
        let value =
            postcard::from_bytes(&stored.value).map_err(|_| PersistenceError::PayloadDecode)?;
        Ok(StoredRecord {
            meta: stored.meta,
            value,
        })
    }

    pub fn save<T: Serialize + ?Sized>(
        &mut self,
        motion: &Motion,
        lifecycle: &Lifecycle,
        spec: RecordSpec<'_>,
        value: &T,
    ) -> Result<RecordMeta, PersistenceError> {
        let payload = postcard::to_allocvec(value).map_err(|_| PersistenceError::PayloadEncode)?;
        self.save_bytes(motion, lifecycle, spec, &payload)
    }

    pub fn into_inner(self) -> Store {
        self.store
    }
}

fn slot_keys(base: &str) -> Result<(String<MAX_NAME_LEN>, String<MAX_NAME_LEN>), StorageError> {
    if base.len() + 1 > MAX_NAME_LEN {
        return Err(StorageError::NameTooLong);
    }
    let mut a = String::new();
    let mut b = String::new();
    write!(&mut a, "{base}0").map_err(|_| StorageError::NameTooLong)?;
    write!(&mut b, "{base}1").map_err(|_| StorageError::NameTooLong)?;
    Ok((a, b))
}
