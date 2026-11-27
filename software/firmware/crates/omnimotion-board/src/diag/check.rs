//! The wiring self-check: reads the pins and drivers and hands them to
//! `omnimotion::devices::check` to judge.

use omnimotion::devices::check::{
    read_endstops, BoardReport, DriverCheck, DriverResult, DriverSamples,
};
use omnimotion::devices::power::PowerSource;
use omnimotion::devices::tmc2209::{Driver, ADDRESSES};
use omnimotion::AXES;

use crate::diag::inventory::{self, DriverPresence};
use crate::motion::supervisor::endstop_levels;
use crate::Board;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckError {
    /// The check toggles STEP, which would move an energised motor.
    DriversEnabled,
    /// A move or abort is still using the step lines.
    MotionActive,
}

impl Board {
    /// Checks the wiring without energising a motor: power, e-stop, OLED,
    /// endstop idle levels, and for each driver that answers, that its STEP,
    /// DIR, DIAG, ENN and address straps reach it and that it reports no fault.
    ///
    /// Needs the outputs disabled, which `Board::new` leaves them. Each STEP
    /// line gets one rising edge; with ENN inactive that moves nothing. Drivers
    /// can only answer on bulk power, so on USB they are reported unchecked.
    /// Neither the inventory nor the set of available axes is changed.
    pub async fn check(&mut self) -> Result<BoardReport, CheckError> {
        if self.faults.drivers_enabled() {
            return Err(CheckError::DriversEnabled);
        }

        let rails = self.faults.rails();
        let mut drivers = [DriverResult::NoMotorSupply; AXES];
        if rails.source() != PowerSource::UsbOnly {
            for (axis, result) in drivers.iter_mut().enumerate() {
                *result = self.check_driver(axis).await?;
            }
        }

        let endstop_config = core::array::from_fn(|endstop| {
            self.faults
                .endstop_config(endstop)
                .expect("endstop in board range")
        });
        Ok(BoardReport {
            rails,
            estop_closed: self.faults.estop_closed(),
            oled: inventory::find_oled(&mut self.i2c).await,
            endstop_config,
            endstops: read_endstops(&endstop_config, endstop_levels()),
            drivers,
        })
    }

    async fn check_driver(&mut self, axis: usize) -> Result<DriverResult, CheckError> {
        let address = ADDRESSES[axis];
        let Board {
            motion,
            tmc,
            faults,
            ..
        } = self;
        match inventory::probe(tmc, address).await {
            DriverPresence::Responding => {}
            DriverPresence::Mismatched(version) => return Ok(DriverResult::Mismatched(version)),
            DriverPresence::Missing => return Ok(DriverResult::Silent),
        }
        let mut driver = Driver::new(tmc, address);

        let mut lines = motion.hold_lines(axis).ok_or(CheckError::MotionActive)?;
        lines.drive(false, false);
        let low = driver.inputs().await;
        let diag_low = faults.diag_asserted(axis);
        lines.drive(true, true);
        let high = driver.inputs().await;
        let diag_high = faults.diag_asserted(axis);
        drop(lines);

        let global = driver.global_status().await;
        let status = driver.status().await;
        let (Ok(low), Ok(high), Ok(global), Ok(status)) = (low, high, global, status) else {
            return Ok(DriverResult::Intermittent);
        };
        Ok(DriverResult::Checked(DriverCheck::judge(
            address,
            &DriverSamples {
                low,
                high,
                mcu_diag: [diag_low, diag_high],
                global,
                status,
            },
        )))
    }
}
