//! The `web` partition: gzipped UI assets written by `tools/pack_web.py`.
//!
//! Each file is one blob in the `web` namespace, keyed by its file name. The
//! partition has its own handle, so reading the UI never touches the settings
//! store, and every read takes the same flash lock as the rest.

extern crate alloc;

use alloc::vec::Vec;

use esp_hal::peripherals::FLASH;
use esp_nvs::Nvs;
use esp_storage::FlashStorage;

use crate::storage::flash;
use crate::storage::{self, StorageError};

pub(crate) const PARTITION_LABEL: &str = "web";
const NAMESPACE: &str = "web";

pub struct WebAssets {
    nvs: Nvs<FlashStorage<'static>>,
}

impl WebAssets {
    /// Opens the `web` partition.
    pub fn new() -> Result<Self, StorageError> {
        // Safety: `Store` and `Ota` hold the other handles, and every handle
        // reaches the chip through `flash::exclusive` alone.
        let flash = unsafe { FLASH::steal() };
        flash::exclusive(|| {
            let mut storage = FlashStorage::new(flash).multicore_auto_park();
            let (offset, size) = storage::nvs_partition(&mut storage, PARTITION_LABEL)?;

            Nvs::new(offset, size, storage)
                .map(|nvs| Self { nvs })
                .map_err(StorageError::Nvs)
        })
    }

    /// The gzipped bytes of the file `name`, or `Ok(None)` when it was never packed.
    pub fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, StorageError> {
        storage::read_nvs_blob(&mut self.nvs, NAMESPACE, name)
    }
}
