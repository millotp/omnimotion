//! A three-axis coordinated move. Every axis starts and finishes together,
//! because one Bresenham accumulator drives all of them from a single tick.

#![no_std]
#![no_main]

use embassy_futures::select::{select, Either};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_println::println;
use omnimotion_board::motion::{Motion, Outcome};
use omnimotion_board::{
    steps_per_mm, AxisConfig, Board, BoardPins, Config, DriverConfig, EndstopConfig,
    MicroStepResolution, AXES, ENDSTOPS,
};

esp_bootloader_esp_idf::esp_app_desc!();

const MICROSTEPS: u32 = 16;

fn config() -> Config {
    let linear = AxisConfig {
        steps_per_unit: steps_per_mm(200, MICROSTEPS, 40.0).expect("valid axis calibration"),
        max_velocity: 60.0,
        max_acceleration: 600.0,
        min_velocity: 2.0,
    };

    // A rotary axis in degrees, geared 5:1.
    let rotary = AxisConfig {
        steps_per_unit: (200 * MICROSTEPS * 5) as f32 / 360.0,
        max_velocity: 180.0,
        max_acceleration: 1_800.0,
        min_velocity: 6.0,
    };

    Config {
        axes: [linear, linear, rotary],
        endstops: [EndstopConfig::SWITCH; ENDSTOPS],
        drivers: [DriverConfig {
            run_current_ma: 700.0,
            hold_current_ma: 350.0,
            microsteps: MicroStepResolution::from_microsteps(MICROSTEPS),
            ..DriverConfig::default()
        }; AXES],
    }
}

async fn run(motion: &mut Motion) {
    loop {
        for target in [[40.0, 10.0, 90.0], [0.0, 0.0, 0.0]] {
            if !matches!(motion.move_to_units(target).await, Ok(Outcome::Completed)) {
                return;
            }
            println!("at {:?}", motion.position_units());
            Timer::after(Duration::from_millis(500)).await;
        }
    }
}

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
    let mut board = Board::start(pins, config()).await.expect("board startup");
    println!("{}", board.inventory);

    // One accumulator drives all three axes together, so unlike the other
    // examples this one genuinely needs every socket populated.
    let responding = board.inventory.drivers_responding();
    if responding != AXES {
        board.faults.disable_drivers();
        panic!("a coordinated move needs three drivers, {responding} answered");
    }

    match select(run(&mut board.motion), board.faults.wait_for_fault()).await {
        Either::First(()) => println!("motion stopped"),
        Either::Second(fault) => println!("fault: {fault:?}"),
    }
    board.faults.disable_drivers();
}
