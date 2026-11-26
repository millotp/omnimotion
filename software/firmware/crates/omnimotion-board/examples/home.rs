//! Homing both ways: M0 sensorless against the frame, M1 against a switch.
//!
//! StallGuard tuning order: start with `threshold` high, lower it until the axis
//! stops reliably at the frame without tripping during the ramp, then adjust
//! `current_ma` down for more sensitivity.
//!
//! Switch homing needs no tuning, but the connector must be declared
//! `EndstopRole::Reference`. A `Limit` would latch `Fault::Endstop` the instant
//! the pass succeeded.

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_println::println;
use omnimotion_board::diag::inventory::DriverPresence;
use omnimotion_board::motion::homing::{home, Homed, HomingConfig, HomingSensor};
use omnimotion_board::{
    steps_per_mm, AxisConfig, Board, BoardPins, Config, DriverConfig, EndstopConfig,
    MicroStepResolution, AXES,
};

esp_bootloader_esp_idf::esp_app_desc!();

const MICROSTEPS: u32 = 16;
const RUN_CURRENT_MA: f32 = 700.0;

/// M1 homes against E2 (J13).
const M1_HOME: usize = 2;

/// Each connector is declared independently, so a machine can run hall sensors
/// where swarf would kill a contact and switches everywhere else. `UNUSED`
/// ignores the pin, which is what an axis without a far-end stop wants.
fn config() -> Config {
    Config {
        axes: [AxisConfig {
            steps_per_unit: steps_per_mm(200, MICROSTEPS, 40.0).expect("valid axis calibration"),
            max_velocity: 60.0,
            max_acceleration: 600.0,
            min_velocity: 2.0,
        }; AXES],
        endstops: [
            EndstopConfig::HALL,             // E0: a limit; M0 homes sensorless
            EndstopConfig::UNUSED,           // E1
            EndstopConfig::SWITCH_REFERENCE, // E2: M1's datum, driven into on purpose
            EndstopConfig::UNUSED,           // E3
            EndstopConfig::SWITCH,           // E4
            EndstopConfig::SWITCH,           // E5
        ],
        driver: DriverConfig {
            run_current_ma: RUN_CURRENT_MA,
            hold_current_ma: RUN_CURRENT_MA / 2.0,
            microsteps: MicroStepResolution::from_microsteps(MICROSTEPS),
            ..DriverConfig::default()
        },
        ..Config::default()
    }
}

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let (rtos, pins) = omnimotion_board::split!(peripherals);
    // Nothing else competes for time here, so the board may keep the first core.
    omnimotion_board::boot(rtos, |spawner| {
        spawner.spawn(board_task(pins).expect("board task spawned once"))
    });
}

#[embassy_executor::task]
async fn board_task(pins: BoardPins) {
    let config = config();
    let mut board = Board::start(pins, config).await.expect("board startup");
    println!("{}", board.inventory);

    // Everything but the sensor is shared: the pass geometry does not care how
    // the stop announces itself. Distances are millimetres, like every move.
    let pass = HomingConfig {
        sensor: HomingSensor::StallGuard {
            current_ma: 450.0,
            threshold: 90,
        },
        travel: -200.0,
        rate: 20.0,
        accel: 200.0,
        backoff: 3.0,
    };

    home_axis(&mut board, 0, &pass, &config.driver).await;

    if board.inventory.drivers[1] == DriverPresence::Responding {
        let switched = HomingConfig {
            // No driver reconfiguration and no settle wait, so this pass runs at
            // the axis's normal current and starts immediately.
            sensor: HomingSensor::Endstop { endstop: M1_HOME },
            ..pass
        };
        home_axis(&mut board, 1, &switched, &config.driver).await;
    } else {
        println!("M1 absent, skipping the switch-homed axis");
    }

    board.faults.disable_drivers();
}

async fn home_axis(
    board: &mut Board,
    axis: usize,
    config: &HomingConfig,
    run_config: &DriverConfig,
) {
    // Built straight from the field rather than `board.driver`, which would
    // borrow the whole board and lock out `motion` and `faults` below.
    let address = omnimotion_board::tmc2209::ADDRESSES[axis];
    let mut driver = omnimotion_board::tmc2209::Driver::new(&mut board.tmc, address);
    let outcome = home(
        &mut board.motion,
        &mut driver,
        &mut board.faults,
        axis,
        config,
        run_config,
    )
    .await;

    match outcome {
        Ok(Homed::Found) => println!(
            "M{axis} homed, now at {:?} mm",
            board.motion.position_units()
        ),
        Ok(Homed::NotFound) => println!("M{axis} ran its whole travel without finding the stop"),
        Err(error) => println!("M{axis} homing failed: {error:?}"),
    }
}
