//! Joins WiFi on core 0 and accepts firmware updates while the board runs on
//! core 1. Build with OMNIMOTION_WIFI_SSID, OMNIMOTION_WIFI_PASSWORD and
//! OMNIMOTION_OTA_TOKEN set; `tools/ota.py` sends later builds over the air.

#![no_std]
#![no_main]

use embassy_net::Stack;
use esp_backtrace as _;
use esp_println::println;
use esp_rtos::embassy::Executor;
use omnimotion_board::{net, storage, Board, BoardPins, Config};
use static_cell::StaticCell;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_hal::main]
fn main() -> ! {
    storage::init_heap();
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let (rtos, pins) = omnimotion_board::split!(peripherals);
    let wifi = peripherals.WIFI;
    omnimotion_board::boot_on_second_core(rtos, |spawner| {
        spawner.spawn(board_task(pins).expect("board task spawned once"))
    });

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    EXECUTOR.init(Executor::new()).run(|spawner| {
        let stack = net::start(spawner, wifi);
        spawner.spawn(report(stack).expect("report task spawned once"));
    })
}

#[embassy_executor::task]
async fn report(stack: Stack<'static>) {
    stack.wait_config_up().await;
    if let Some(config) = stack.config_v4() {
        println!(
            "listening for updates on {}:{}",
            config.address.address(),
            net::OTA_PORT
        );
    }
}

/// `Board::new` leaves the outputs disabled, so this runs on USB power alone.
#[embassy_executor::task]
async fn board_task(pins: BoardPins) {
    let board = Board::new(pins, &Config::default()).await;
    println!("{}", board.inventory);
}
