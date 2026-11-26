//! Checks the board's wiring without energising a motor. Flash this during
//! bring-up and after any rewiring; fill in `Config::endstops` to match what is
//! plugged into J11-J16 so their idle levels are judged correctly.

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_println::println;
use omnimotion_board::{Board, BoardPins, Config};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let (rtos, pins) = omnimotion_board::split!(peripherals);
    omnimotion_board::boot(rtos, |spawner| {
        spawner.spawn(board_task(pins).expect("board task spawned once"))
    });
}

#[embassy_executor::task]
async fn board_task(pins: BoardPins) {
    let mut board = Board::new(pins, &Config::default()).await;
    match board.check().await {
        Ok(report) => println!("{report}"),
        Err(error) => println!("check refused: {error:?}"),
    }
}
