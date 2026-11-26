//! Persistent storage in the ESP-IDF `nvs` partition.
//!
//! `esp-nvs` does the work. This wraps the parts every consumer would
//! otherwise repeat: locating the partition rather than hardcoding its offset,
//! and serialising flash access with the other partition handles.
//!
//! The NVS format caps keys and namespaces at 15 bytes and `esp-nvs` panics on
//! anything longer, so names are checked here and rejected as an error.
//!
//! NVS allocates while it walks its pages, so a global allocator has to exist
//! before [`Store::new`]. [`init_heap`] installs one for applications that do
//! not already have their own.

pub(crate) mod flash;
pub mod persistence;
pub mod web;

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use esp_bootloader_esp_idf::partitions::{
    self, DataPartitionSubType, PartitionType, PARTITION_TABLE_MAX_LEN,
};
use esp_hal::peripherals::FLASH;
use esp_nvs::error::Error as NvsError;
use esp_nvs::{Key, Nvs};
use esp_storage::FlashStorage;

/// The NVS limit on a key or namespace, excluding the null terminator.
pub const MAX_NAME_LEN: usize = 15;

/// Room for the page cache plus one blob in flight. Applications storing
/// anything larger should install their own allocator instead of `init_heap`.
pub const DEFAULT_HEAP: usize = 96 * 1024;

/// RAM the bootloader used and hands back, which esp-radio needs for its WiFi
/// buffers on top of what NVS takes.
const RECLAIMED_HEAP: usize = 64 * 1024;

/// The settings partition in `partitions.csv`. The web assets live in their own
/// `web` partition, so reflashing the UI never erases settings.
const PARTITION_LABEL: &str = "nvs";

/// Installs a global allocator. Call once, before [`Store::new`] or
/// [`crate::net::start`].
pub fn init_heap() {
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: RECLAIMED_HEAP);
    esp_alloc::heap_allocator!(size: DEFAULT_HEAP);
}

#[derive(Debug)]
pub enum StorageError {
    /// The partition table has no NVS data partition with the requested label.
    NoNvsPartition,
    /// A key or namespace was longer than [`MAX_NAME_LEN`].
    NameTooLong,
    PartitionTable(partitions::Error),
    Nvs(NvsError),
}

/// A handle on the NVS partition. One per board; `esp-nvs` keeps a page cache
/// that is expensive to rebuild, so hold onto it rather than reopening.
pub struct Store {
    nvs: Nvs<FlashStorage<'static>>,
}

impl Store {
    /// Finds the `nvs` partition in the table written by `espflash` and opens it.
    ///
    /// Writes park the other core for the duration of the erase, since it must
    /// not fetch from flash meanwhile. On the board's core that stalls the
    /// supervisor too, which `TransactionalStore` keeps to idle motion.
    pub fn new(flash: FLASH<'static>) -> Result<Self, StorageError> {
        flash::exclusive(|| {
            let mut storage = FlashStorage::new(flash).multicore_auto_park();

            let (offset, size) = nvs_partition(&mut storage, PARTITION_LABEL)?;

            Nvs::new(offset, size, storage)
                .map(|nvs| Self { nvs })
                .map_err(StorageError::Nvs)
        })
    }

    /// Raw bytes, for content that is already encoded. Blobs may span pages and
    /// run to roughly 500 kB.
    pub fn write_blob(
        &mut self,
        namespace: &str,
        key: &str,
        bytes: &[u8],
    ) -> Result<(), StorageError> {
        let (namespace, key) = names(namespace, key)?;
        flash::exclusive(|| self.nvs.set(&namespace, &key, bytes)).map_err(StorageError::Nvs)
    }

    /// `Ok(None)` when nothing has been stored under this key yet.
    pub fn read_blob(
        &mut self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        read_nvs_blob(&mut self.nvs, namespace, key)
    }

    /// Removing a key that was never written is not an error.
    pub fn delete(&mut self, namespace: &str, key: &str) -> Result<(), StorageError> {
        let (namespace, key) = names(namespace, key)?;
        flash::exclusive(|| self.nvs.delete(&namespace, &key)).map_err(StorageError::Nvs)
    }
}

pub(crate) fn names(namespace: &str, key: &str) -> Result<(Key, Key), StorageError> {
    if namespace.len() > MAX_NAME_LEN || key.len() > MAX_NAME_LEN {
        return Err(StorageError::NameTooLong);
    }
    Ok((Key::from_str(namespace), Key::from_str(key)))
}

/// Offset and size of the NVS partition labelled `label` in the table on `storage`.
pub(crate) fn nvs_partition(
    storage: &mut FlashStorage<'_>,
    label: &str,
) -> Result<(usize, usize), StorageError> {
    let mut buf = vec![0u8; PARTITION_TABLE_MAX_LEN];
    let table = partitions::read_partition_table(storage, &mut buf)
        .map_err(StorageError::PartitionTable)?;
    let entry = table
        .iter()
        .find(|entry| {
            entry.partition_type() == PartitionType::Data(DataPartitionSubType::Nvs)
                && entry.label_as_str() == label
        })
        .ok_or(StorageError::NoNvsPartition)?;
    Ok((entry.offset() as usize, entry.len() as usize))
}

/// `Ok(None)` when nothing has been stored under this key yet.
pub(crate) fn read_nvs_blob(
    nvs: &mut Nvs<FlashStorage<'_>>,
    namespace: &str,
    key: &str,
) -> Result<Option<Vec<u8>>, StorageError> {
    let (namespace, key) = names(namespace, key)?;
    match flash::exclusive(|| nvs.get::<Vec<u8>>(&namespace, &key)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(NvsError::KeyNotFound | NvsError::NamespaceNotFound) => Ok(None),
        Err(error) => Err(StorageError::Nvs(error)),
    }
}
