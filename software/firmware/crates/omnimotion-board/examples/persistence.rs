//! Writes and reloads a versioned settings record while motion is idle.
//!
//! Settings need no motors, so this builds the board with `Board::new` and never
//! enables the outputs, which also leaves the watchdog unarmed.

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::peripherals::FLASH;
use esp_println::println;
use omnimotion::{Lifecycle, RecordKind};
use omnimotion_board::storage::persistence::{RecordSpec, StoredRecord, TransactionalStore};
use omnimotion_board::storage::{self, Store};
use omnimotion_board::{Board, BoardPins, Config};
use serde::{Deserialize, Serialize};

esp_bootloader_esp_idf::esp_app_desc!();

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct Settings {
    speed_percent: u8,
}

#[esp_hal::main]
fn main() -> ! {
    storage::init_heap();
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let (rtos, pins) = omnimotion_board::split!(peripherals);
    let flash = peripherals.FLASH;
    omnimotion_board::boot(rtos, |spawner| {
        spawner.spawn(board_task(pins, flash).expect("board task spawned once"))
    });
}

#[embassy_executor::task]
async fn board_task(pins: BoardPins, flash: FLASH<'static>) {
    let board = Board::new(pins, &Config::default()).await;
    let mut records =
        TransactionalStore::new(Store::new(flash).expect("NVS partition is available"));
    let lifecycle = Lifecycle::new();
    let settings = RecordSpec {
        namespace: "machine",
        base_key: "settings",
        kind: RecordKind::SETTINGS,
        schema_version: 1,
    };

    records
        .save(
            &board.motion,
            &lifecycle,
            settings,
            &Settings { speed_percent: 75 },
        )
        .expect("idle settings write");
    let loaded: StoredRecord<Settings> = records.load(settings).expect("settings load");
    println!(
        "settings generation {}, value {:?}",
        loaded.meta.generation, loaded.value
    );
}
