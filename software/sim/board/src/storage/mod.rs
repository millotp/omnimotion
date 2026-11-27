//! The `nvs` partition, held by the simulated flash (it survives resets). The flash lock and the
//! transactional persistence layer are the production sources.

// Only the flash lock is used; the simulated NVS replaces the production flash leases.
#[allow(dead_code)]
#[path = "../../../../firmware/crates/omnimotion-board/src/storage/flash.rs"]
pub(crate) mod flash;
#[path = "../../../../firmware/crates/omnimotion-board/src/storage/persistence.rs"]
pub mod persistence;

use alloc::vec::Vec;

use esp_hal::peripherals::FLASH;
pub use omnimotion::machine::persistence::{names_fit, MAX_NAME_LEN};

/// The host's allocator is already in place.
pub fn init_heap() {}

#[derive(Debug)]
pub enum StorageError {
    NameTooLong,
}

pub struct Store(());

impl Store {
    pub fn new(_flash: FLASH<'static>) -> Result<Self, StorageError> {
        Ok(Self(()))
    }

    pub fn write_blob(
        &mut self,
        namespace: &str,
        key: &str,
        bytes: &[u8],
    ) -> Result<(), StorageError> {
        names(namespace, key)?;
        flash::exclusive(|| omnimotion_sim::hal::nvs_set(namespace, key, bytes));
        Ok(())
    }

    pub fn read_blob(
        &mut self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        names(namespace, key)?;
        Ok(flash::exclusive(|| {
            omnimotion_sim::hal::nvs_get(namespace, key)
        }))
    }
}

fn names(namespace: &str, key: &str) -> Result<(), StorageError> {
    if !names_fit(namespace, key) {
        return Err(StorageError::NameTooLong);
    }
    Ok(())
}
