//! Runs motion on core 1 and requests an in-flight abort from core 0.

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_futures::select::{select, Either};
use esp_backtrace as _;
use esp_hal::delay::Delay;
use esp_println::println;
use omnimotion_board::motion::AbortHandle;
use omnimotion_board::{Board, BoardPins, Config};

esp_bootloader_esp_idf::esp_app_desc!();

static MOTION_READY: AtomicBool = AtomicBool::new(false);

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let (rtos, pins) = omnimotion_board::split!(peripherals);
    omnimotion_board::boot_on_second_core(rtos, |spawner| {
        spawner.spawn(board_task(pins).expect("board task spawned once"))
    });

    let delay = Delay::new();
    while !MOTION_READY.load(Ordering::Acquire) {
        delay.delay_millis(10);
    }

    delay.delay_millis(500);
    let abort = AbortHandle;
    let ticket = abort.request();
    while !abort.state().is_acknowledged(ticket) {
        delay.delay_millis(1);
    }
    println!("core 0 observed abort acknowledgement at {:?}", ticket);

    loop {
        delay.delay_millis(5_000);
    }
}

#[embassy_executor::task]
async fn board_task(pins: BoardPins) {
    let mut board = Board::start(pins, Config::default())
        .await
        .expect("board startup");
    MOTION_READY.store(true, Ordering::Release);

    let motion = board.motion.move_by([1_000_000, 0, 0]);
    match select(motion, board.faults.wait_for_fault()).await {
        Either::First(result) => println!("motion result: {:?}", result),
        Either::Second(fault) => println!("fault: {fault:?}"),
    }
    board.faults.disable_drivers();
}
