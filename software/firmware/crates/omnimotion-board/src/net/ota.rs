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
use omnimotion::update::ota::{check_image_header, ImageError, Upload};
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
    /// An app image larger than its slot.
    TooLarge,
    Image(ImageError),
    /// A web image that is not the size of the web partition.
    WrongSize,
    /// A web image page is not NVS, so it was not written. Earlier pages
    /// already were.
    NotAnNvsPage,
    /// The body ended before the length it announced.
    Truncated,
    /// The table lacks otadata, the OTA slots or the web partition, or otadata
    /// is unreadable.
    Partitions,
    Flash,
}

impl OtaError {
    pub(crate) fn response(&self) -> (u16, &'static str) {
        match self {
            Self::Busy => (409, "a move is running; stop it first"),
            Self::TooLarge => (413, "image larger than its slot"),
            Self::Image(ImageError::WrongChip(_)) => (422, "image built for another chip"),
            Self::Image(_) => (422, "not an app image; send espflash save-image output"),
            Self::WrongSize => (
                422,
                "not the size of the web partition; send pack_web.py output",
            ),
            Self::NotAnNvsPage => (422, "not NVS pages; send pack_web.py output"),
            Self::Truncated => (400, "body shorter than Content-Length"),
            Self::Partitions | Self::Flash => (500, "flash error"),
        }
    }
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

    /// Claims the inactive app slot, or the web partition for an image of
    /// exactly its size. Moves fail with `Busy` until the returned `Update` is
    /// dropped.
    pub(crate) fn begin(&mut self, upload: Upload) -> Result<Update<'_>, OtaError> {
        let lease = Lease::try_upload().ok_or(OtaError::Busy)?;
        let Self { flash, table, .. } = self;
        let length = upload.length();
        let (target, offset) = match upload {
            Upload::App(_) => {
                if length == 0 {
                    return Err(OtaError::Image(ImageError::TooShort));
                }
                let (slot, offset, capacity) = flash::exclusive(|| {
                    let table = partitions::read_partition_table(flash, table)?;
                    let slot = match booted(&table)? {
                        AppPartitionSubType::Ota0 => AppPartitionSubType::Ota1,
                        _ => AppPartitionSubType::Ota0,
                    };
                    let entry = table
                        .find_partition(PartitionType::App(slot))?
                        .ok_or(partitions::Error::Invalid)?;
                    Ok::<_, partitions::Error>((slot, entry.offset(), entry.len() as usize))
                })?;
                if length > capacity {
                    return Err(OtaError::TooLarge);
                }
                (Target::App(slot), offset)
            }
            Upload::Web(_) => {
                let (offset, size) =
                    flash::exclusive(|| storage::nvs_partition(flash, PARTITION_LABEL))
                        .map_err(|_| OtaError::Partitions)?;
                if length != size {
                    return Err(OtaError::WrongSize);
                }
                (Target::Web, offset as u32)
            }
        };
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

#[derive(Clone, Copy)]
enum Target {
    App(AppPartitionSubType),
    Web,
}

/// An image being streamed into its partition, one erased sector at a time.
pub(crate) struct Update<'a> {
    ota: &'a mut Ota,
    _lease: Lease,
    target: Target,
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

    /// An app image is selected for the next boot, as `New` so it gets a trial.
    pub(crate) fn finish(self) -> Result<(), OtaError> {
        if self.received != self.length {
            return Err(OtaError::Truncated);
        }
        let Target::App(slot) = self.target else {
            return Ok(());
        };
        self.ota.with_otadata(|otadata, booted| {
            // Blank otadata reads as the factory slot, which this table lacks.
            // Pinning the running slot first keeps the sequence arithmetic sane.
            if otadata.current_app_partition()? == AppPartitionSubType::Factory {
                otadata.set_current_app_partition(booted)?;
            }
            otadata.set_current_app_partition(slot)?;
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
        match self.target {
            Target::App(_) if start == 0 => {
                check_image_header(&sector[..filled]).map_err(OtaError::Image)?
            }
            Target::App(_) => {}
            Target::Web if !is_nvs_page(&sector[..filled]) => return Err(OtaError::NotAnNvsPage),
            Target::Web => {}
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
