//! Firmware images written to the inactive app slot, and the trial that decides
//! whether a new image is kept. Web partition images are written here too,
//! since they go through the same flash handle and lease.
//!
//! espflash's bundled bootloader has no rollback support, but it does skip a
//! slot whose otadata entry reads `Invalid`. So the trial lives here: a new
//! image boots as `New` and marks itself `PendingVerify`, and `confirm` makes it
//! `Valid`. Booting into `PendingVerify` means the previous attempt never got
//! that far, so the image marks itself `Invalid` and resets into the old slot.
//! A bootloader built with CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE sets
//! `PendingVerify` itself and would make every update look failed.

use embedded_storage::nor_flash::NorFlash;
use esp_bootloader_esp_idf::ota::{Ota as OtaData, OtaImageState};
use esp_bootloader_esp_idf::partitions::{
    self, AppPartitionSubType, DataPartitionSubType, PartitionTable, PartitionType,
    PARTITION_TABLE_MAX_LEN,
};
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;
use omnimotion::update::ota::{check_image_header, ImageError};
use omnimotion::update::web::is_nvs_page;

use crate::storage;
use crate::storage::flash::{self, Lease};
use crate::storage::web::PARTITION_LABEL;

const SECTOR: usize = FlashStorage::SECTOR_SIZE as usize;
const WORD: usize = FlashStorage::WORD_SIZE as usize;
/// `ota_0` and `ota_1` in `partitions.csv`.
const OTA_SLOTS: usize = 2;

#[derive(Debug)]
pub(crate) enum OtaError {
    /// A move is running, or another update holds the flash.
    Busy,
    TooLarge,
    Image(ImageError),
    /// The body ended before the length it announced.
    Truncated,
    /// The table lacks otadata or the OTA slots, or otadata is unreadable.
    Partitions,
    Flash,
}

impl From<partitions::Error> for OtaError {
    fn from(_: partitions::Error) -> Self {
        Self::Partitions
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Boot {
    /// Flashed over USB, or already confirmed.
    Settled,
    /// Freshly installed; call `confirm` once it has shown it can be updated again.
    Trial,
    /// Its trial ended in a reset, so it is now marked invalid. Resetting
    /// returns to the previous image.
    Failed,
}

pub(crate) struct Ota {
    flash: FlashStorage<'static>,
    table: [u8; PARTITION_TABLE_MAX_LEN],
    sector: [u8; SECTOR],
}

impl Ota {
    /// One per board, alongside `Store`.
    pub(crate) fn new() -> Self {
        // Safety: `Store` holds the only other handle, and both reach the chip
        // through `flash::exclusive` alone.
        let flash = unsafe { FLASH::steal() };
        Self {
            flash: flash::exclusive(|| FlashStorage::new(flash).multicore_auto_park()),
            table: [0; PARTITION_TABLE_MAX_LEN],
            sector: [0; SECTOR],
        }
    }

    /// Run once at boot, before anything else could reset the board.
    pub(crate) fn start_trial(&mut self, _lease: &Lease) -> Result<Boot, OtaError> {
        self.with_otadata(|otadata, booted| {
            if otadata.current_app_partition()? != booted {
                return Ok(Boot::Settled);
            }
            match otadata.current_ota_state() {
                Ok(OtaImageState::New) => {
                    otadata.set_current_ota_state(OtaImageState::PendingVerify)?;
                    Ok(Boot::Trial)
                }
                Ok(OtaImageState::PendingVerify) => {
                    otadata.set_current_ota_state(OtaImageState::Invalid)?;
                    Ok(Boot::Failed)
                }
                _ => Ok(Boot::Settled),
            }
        })
    }

    pub(crate) fn confirm(&mut self, _lease: &Lease) -> Result<(), OtaError> {
        self.mark_booted(OtaImageState::Valid)
    }

    /// The previous image boots after the next reset.
    pub(crate) fn reject(&mut self, _lease: &Lease) -> Result<(), OtaError> {
        self.mark_booted(OtaImageState::Invalid)
    }

    /// Claims the web partition for an image of exactly its size. Moves fail
    /// with `Busy` until the returned `WebUpdate` is dropped.
    pub(crate) fn begin_web(&mut self, length: usize) -> Result<WebUpdate<'_>, WebError> {
        let lease = Lease::try_upload().ok_or(WebError::Busy)?;
        let Self { flash, .. } = self;
        let (offset, size) = flash::exclusive(|| storage::nvs_partition(flash, PARTITION_LABEL))
            .map_err(|_| WebError::Partitions)?;
        if length != size {
            return Err(WebError::WrongSize);
        }
        Ok(WebUpdate {
            ota: self,
            _lease: lease,
            offset: offset as u32,
            length,
            received: 0,
        })
    }

    /// Claims the inactive slot for an image of `length` bytes. Moves fail
    /// with `Busy` until the returned `Update` is dropped.
    pub(crate) fn begin(&mut self, length: usize) -> Result<Update<'_>, OtaError> {
        if length == 0 {
            return Err(OtaError::Image(ImageError::TooShort));
        }
        let lease = Lease::try_upload().ok_or(OtaError::Busy)?;
        let Self { flash, table, .. } = self;
        let (target, offset, capacity) = flash::exclusive(|| {
            let table = partitions::read_partition_table(flash, table)?;
            let target = match booted(&table)? {
                AppPartitionSubType::Ota0 => AppPartitionSubType::Ota1,
                _ => AppPartitionSubType::Ota0,
            };
            let entry = table
                .find_partition(PartitionType::App(target))?
                .ok_or(partitions::Error::Invalid)?;
            Ok::<_, partitions::Error>((target, entry.offset(), entry.len() as usize))
        })?;
        if length > capacity {
            return Err(OtaError::TooLarge);
        }
        Ok(Update {
            ota: self,
            _lease: lease,
            target,
            offset,
            length,
            received: 0,
        })
    }

    fn mark_booted(&mut self, state: OtaImageState) -> Result<(), OtaError> {
        self.with_otadata(|otadata, booted| {
            if otadata.current_app_partition()? == booted {
                otadata.set_current_ota_state(state)?;
            }
            Ok(())
        })
    }

    fn with_otadata<R>(
        &mut self,
        operation: impl FnOnce(
            &mut OtaData<'_, FlashStorage<'static>>,
            AppPartitionSubType,
        ) -> Result<R, partitions::Error>,
    ) -> Result<R, OtaError> {
        let Self { flash, table, .. } = self;
        flash::exclusive(|| {
            let table = partitions::read_partition_table(flash, table)?;
            let booted = booted(&table)?;
            let entry = table
                .find_partition(PartitionType::Data(DataPartitionSubType::Ota))?
                .ok_or(partitions::Error::Invalid)?;
            let mut otadata = OtaData::new(entry.as_embedded_storage(flash), OTA_SLOTS)?;
            operation(&mut otadata, booted)
        })
        .map_err(OtaError::from)
    }
}

/// Read from the MMU rather than otadata, which may name a slot the
/// bootloader refused.
fn booted(table: &PartitionTable<'_>) -> Result<AppPartitionSubType, partitions::Error> {
    match table
        .booted_partition()?
        .map(|entry| entry.partition_type())
    {
        Some(PartitionType::App(slot)) => Ok(slot),
        _ => Err(partitions::Error::Invalid),
    }
}

#[derive(Debug)]
pub(crate) enum WebError {
    /// A move is running, or another update holds the flash.
    Busy,
    /// The image is not the size of the web partition.
    WrongSize,
    /// A page is not NVS, so it was not written. Earlier pages already were.
    NotAnNvsPage,
    /// The body ended before the length it announced.
    Truncated,
    Partitions,
    Flash,
}

/// An image being streamed into the web partition, one erased page at a time.
pub(crate) struct WebUpdate<'a> {
    ota: &'a mut Ota,
    _lease: Lease,
    offset: u32,
    length: usize,
    received: usize,
}

impl WebUpdate<'_> {
    pub(crate) fn write(&mut self, mut bytes: &[u8]) -> Result<(), WebError> {
        if bytes.len() > self.length - self.received {
            return Err(WebError::WrongSize);
        }
        while !bytes.is_empty() {
            let filled = self.received % SECTOR;
            let take = bytes.len().min(SECTOR - filled);
            self.ota.sector[filled..filled + take].copy_from_slice(&bytes[..take]);
            self.received += take;
            bytes = &bytes[take..];
            if self.received.is_multiple_of(SECTOR) {
                self.flush()?;
            }
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<(), WebError> {
        if self.received != self.length {
            return Err(WebError::Truncated);
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), WebError> {
        let start = self.received - SECTOR;
        let Ota { flash, sector, .. } = &mut *self.ota;
        if !is_nvs_page(&sector[..]) {
            return Err(WebError::NotAnNvsPage);
        }
        let address = self.offset + start as u32;
        flash::exclusive(|| {
            flash.erase(address, address + SECTOR as u32)?;
            flash.write(address, &sector[..])
        })
        .map_err(|_| WebError::Flash)
    }
}

/// An image being streamed into the inactive slot, one erased sector at a time.
pub(crate) struct Update<'a> {
    ota: &'a mut Ota,
    _lease: Lease,
    target: AppPartitionSubType,
    offset: u32,
    length: usize,
    received: usize,
}

impl Update<'_> {
    pub(crate) fn write(&mut self, mut bytes: &[u8]) -> Result<(), OtaError> {
        if bytes.len() > self.length - self.received {
            return Err(OtaError::TooLarge);
        }
        while !bytes.is_empty() {
            let filled = self.received % SECTOR;
            let take = bytes.len().min(SECTOR - filled);
            self.ota.sector[filled..filled + take].copy_from_slice(&bytes[..take]);
            self.received += take;
            bytes = &bytes[take..];
            if self.received.is_multiple_of(SECTOR) || self.received == self.length {
                self.flush()?;
            }
        }
        Ok(())
    }

    /// Selects the new image for the next boot, as `New` so it gets a trial.
    pub(crate) fn finish(self) -> Result<(), OtaError> {
        if self.received != self.length {
            return Err(OtaError::Truncated);
        }
        let target = self.target;
        self.ota.with_otadata(|otadata, booted| {
            // Blank otadata reads as the factory slot, which this table lacks.
            // Pinning the running slot first keeps the sequence arithmetic sane.
            if otadata.current_app_partition()? == AppPartitionSubType::Factory {
                otadata.set_current_app_partition(booted)?;
            }
            otadata.set_current_app_partition(target)?;
            otadata.set_current_ota_state(OtaImageState::New)
        })
    }

    fn flush(&mut self) -> Result<(), OtaError> {
        let filled = match self.received % SECTOR {
            0 => SECTOR,
            partial => partial,
        };
        let start = self.received - filled;
        let Ota { flash, sector, .. } = &mut *self.ota;
        if start == 0 {
            check_image_header(&sector[..filled]).map_err(OtaError::Image)?;
        }
        let padded = filled.next_multiple_of(WORD);
        sector[filled..padded].fill(0xFF);
        let address = self.offset + start as u32;
        flash::exclusive(|| {
            flash.erase(address, address + SECTOR as u32)?;
            flash.write(address, &sector[..padded])
        })
        .map_err(|_| OtaError::Flash)
    }
}
