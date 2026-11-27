//! Jogs M0 back and forth. Start here when bringing up a new board.
//!
//! Also shows the two-core split: motion owns the second core, and the first
//! goes back to the application. Swap `boot_on_second_core` for `boot` to put
//! everything on one core instead.

#![no_std]
#![no_main]

use embassy_futures::select::{select, Either};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::delay::Delay;
use esp_println::println;
use omnimotion_board::motion::Motion;
use omnimotion_board::{
    steps_per_mm, AxisConfig, Board, BoardPins, Config, DriverConfig, MicroStepResolution, AXES,
};

esp_bootloader_esp_idf::esp_app_desc!();

const MICROSTEPS: u32 = 16;

fn config() -> Config {
    Config {
        axes: [AxisConfig {
            steps_per_unit: steps_per_mm(200, MICROSTEPS, 40.0).expect("valid axis calibration"),
            max_velocity: 60.0,
            max_acceleration: 600.0,
            min_velocity: 2.0,
        }; AXES],
        drivers: [DriverConfig {
            run_current_ma: 700.0,
            hold_current_ma: 350.0,
            microsteps: MicroStepResolution::from_microsteps(MICROSTEPS),
            ..DriverConfig::default()
        }; AXES],
        ..Config::default()
    }
}

async fn run(motion: &mut Motion) {
    loop {
        for distance in [20.0, -20.0] {
            if motion.jog(0, distance).await.is_err() {
                return;
            }
            println!("at {:?} mm", motion.position_units());
            Timer::after(Duration::from_millis(400)).await;
        }
    }
}

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let (rtos, pins) = omnimotion_board::split!(peripherals);

    // `board_task` builds the `Board`, which installs the tick ISR on whichever
    // core runs it, so this is what pins step generation to the second core.
    omnimotion_board::boot_on_second_core(rtos, |spawner| {
        spawner.spawn(board_task(pins).expect("board task spawned once"))
    });

    let delay = Delay::new();
    loop {
        // Standing in for the application's own work. A real build would start
        // its own executor here, safe in the knowledge that its interrupts
        // cannot disturb the step ISR.
        delay.delay_millis(5_000);
        println!("core 0 idle; motion is running on core 1");
    }
}

#[embassy_executor::task]
async fn board_task(pins: BoardPins) {
    let mut board = Board::start(pins, config()).await.expect("board startup");
    println!("{}", board.inventory);

    match select(run(&mut board.motion), board.faults.wait_for_fault()).await {
        Either::First(()) => println!("motion stopped"),
        Either::Second(fault) => println!("fault: {fault:?}"),
    }
    board.faults.disable_drivers();
}
