//! Judging the board's wiring self-check from what its pins and drivers report.
//!
//! Each TMC2209 reports the level on its own STEP, DIR, ENN, DIAG and MS1/MS2
//! pins in IOIN. Driving the MCU side to a known level and reading it back over
//! UART proves the trace in between without energising a motor.

use core::fmt;

use crate::devices::endstop::{EndstopConfig, EndstopKind, ENDSTOPS};
use crate::devices::power::{PowerSource, Rails, Volts, MOTOR_MAX_MV, MOTOR_MIN_MV};
use crate::devices::tmc2209::reg;
use crate::AXES;

/// One driver's pins as it saw them, read with STEP and DIR driven low and
/// then high, while firmware holds the outputs disabled.
#[derive(Clone, Copy, Debug)]
pub struct DriverSamples {
    pub low: reg::IOIN,
    pub high: reg::IOIN,
    /// The MCU's own DIAG input when each sample was taken.
    pub mcu_diag: [bool; 2],
    pub global: reg::GSTAT,
    pub status: reg::DRV_STATUS,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Temperature {
    Normal,
    Warning,
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverCheck {
    /// MS1/MS2 read back as the address the driver answered on.
    pub straps: bool,
    pub step: bool,
    pub dir: bool,
    /// The driver and the MCU agree on DIAG. Both idle low, so this catches a
    /// stuck or shorted line, not an open one.
    pub diag: bool,
    /// ENN reads inactive while firmware commands the outputs off. Anything
    /// else means Q1 or the ENN net holds the drivers enabled.
    pub enable_off: bool,
    pub charge_pump: bool,
    pub temperature: Temperature,
}

impl DriverCheck {
    pub fn judge(address: u8, samples: &DriverSamples) -> Self {
        let DriverSamples { low, high, .. } = *samples;
        let straps =
            |io: reg::IOIN| io.ms1() == (address & 1 != 0) && io.ms2() == (address & 2 != 0);
        let temperature = if samples.status.ot() {
            Temperature::Shutdown
        } else if samples.status.otpw() {
            Temperature::Warning
        } else {
            Temperature::Normal
        };
        Self {
            straps: straps(low) && straps(high),
            step: !low.step() && high.step(),
            dir: !low.dir() && high.dir(),
            diag: low.diag() == samples.mcu_diag[0] && high.diag() == samples.mcu_diag[1],
            enable_off: low.enn() && high.enn(),
            charge_pump: !samples.global.uv_cp(),
            temperature,
        }
    }

    pub fn passed(&self) -> bool {
        self.straps
            && self.step
            && self.dir
            && self.diag
            && self.enable_off
            && self.charge_pump
            && self.temperature == Temperature::Normal
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverResult {
    /// Without VMOTOR a TMC2209 cannot answer, so nothing was tested.
    NoMotorSupply,
    /// No reply on bulk power: not fitted, or not working.
    Silent,
    Mismatched(u8),
    /// Answered its version, then stopped replying part way through.
    Intermittent,
    Checked(DriverCheck),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndstopReading {
    /// Not configured, and idling high as an empty connector should.
    Empty,
    /// Not configured, yet something holds it low.
    UnexpectedLow,
    Clear,
    /// Triggered at rest: the machine is parked on it, or a normally-closed
    /// cable is cut. A normally-open switch with a cut cable reads `Clear`.
    Triggered,
}

pub fn read_endstops(config: &[EndstopConfig; ENDSTOPS], inputs: u8) -> [EndstopReading; ENDSTOPS] {
    core::array::from_fn(|endstop| {
        let low = inputs & (1 << endstop) == 0;
        let config = config[endstop];
        match (config.kind, low == config.active_low) {
            (EndstopKind::Unused, _) if low => EndstopReading::UnexpectedLow,
            (EndstopKind::Unused, _) => EndstopReading::Empty,
            (_, true) => EndstopReading::Triggered,
            (_, false) => EndstopReading::Clear,
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoardReport {
    pub rails: Rails,
    pub estop_closed: bool,
    pub oled: Option<u8>,
    pub endstop_config: [EndstopConfig; ENDSTOPS],
    pub endstops: [EndstopReading; ENDSTOPS],
    pub drivers: [DriverResult; AXES],
}

impl BoardReport {
    pub fn failures(&self) -> usize {
        let power = usize::from(self.rails.source() == PowerSource::BulkOutOfRange);
        let estop = usize::from(!self.estop_closed);
        let endstops = self
            .endstops
            .iter()
            .filter(|reading| **reading == EndstopReading::UnexpectedLow)
            .count();
        let drivers = self
            .drivers
            .iter()
            .filter(|driver| match driver {
                DriverResult::Mismatched(_) | DriverResult::Intermittent => true,
                DriverResult::Checked(check) => !check.passed(),
                DriverResult::NoMotorSupply | DriverResult::Silent => false,
            })
            .count();
        power + estop + endstops + drivers
    }

    /// Worth a look but possibly intended: a datum the machine is parked on,
    /// or drivers left untested on USB power.
    pub fn warnings(&self) -> usize {
        let endstops = self
            .endstops
            .iter()
            .filter(|reading| **reading == EndstopReading::Triggered)
            .count();
        let untested = usize::from(self.rails.source() == PowerSource::UsbOnly);
        endstops + untested
    }

    pub fn passed(&self) -> bool {
        self.failures() == 0
    }
}

/// The marks a check report puts on each line, for applications that add lines of their own.
pub const OK: &str = "ok  ";
pub const FAIL: &str = "FAIL";
pub const WARN: &str = "warn";
pub const INFO: &str = "--  ";

/// A check report's last line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Totals {
    pub failures: usize,
    pub warnings: usize,
}

impl fmt::Display for Totals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "result: {} failure(s), {} warning(s)",
            self.failures, self.warnings
        )
    }
}

impl fmt::Display for BoardReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "board check")?;

        let volts = Volts(self.rails.motor_mv);
        match self.rails.source() {
            PowerSource::Bulk => writeln!(f, "  power   {OK} bulk {volts}")?,
            PowerSource::UsbOnly => writeln!(
                f,
                "  power   {WARN} USB only; drivers need VMOTOR to be checked"
            )?,
            PowerSource::BulkOutOfRange => writeln!(
                f,
                "  power   {FAIL} bulk {volts}, outside {} to {}",
                Volts(MOTOR_MIN_MV),
                Volts(MOTOR_MAX_MV)
            )?,
        }

        match self.estop_closed {
            true => writeln!(f, "  e-stop  {OK} closed")?,
            false => writeln!(f, "  e-stop  {FAIL} open, or J3 unwired")?,
        }

        match self.oled {
            Some(address) => writeln!(f, "  oled    {INFO} {address:#04x} responding")?,
            None => writeln!(f, "  oled    {INFO} not fitted")?,
        }

        for (endstop, reading) in self.endstops.iter().enumerate() {
            let kind = match self.endstop_config[endstop].kind {
                EndstopKind::Unused => "unused",
                EndstopKind::Switch => "switch",
                EndstopKind::Hall => "hall",
            };
            let (mark, detail) = match reading {
                EndstopReading::Empty => (INFO, "idle"),
                EndstopReading::UnexpectedLow => (FAIL, "held low; wire it in Config or unplug it"),
                EndstopReading::Clear => (OK, "clear"),
                EndstopReading::Triggered => {
                    (WARN, "triggered at rest; parked on it, or a cut NC cable")
                }
            };
            writeln!(f, "  E{endstop}      {mark} {kind}, {detail}")?;
        }

        for (axis, driver) in self.drivers.iter().enumerate() {
            write!(f, "  M{axis}      ")?;
            match driver {
                DriverResult::NoMotorSupply => writeln!(f, "{INFO} not checked, no VMOTOR")?,
                DriverResult::Silent => writeln!(f, "{INFO} silent, not fitted?")?,
                DriverResult::Mismatched(version) => writeln!(
                    f,
                    "{FAIL} answered with version {version:#04x}, not a TMC2209"
                )?,
                DriverResult::Intermittent => {
                    writeln!(f, "{FAIL} answered, then stopped; check the UART lines")?
                }
                DriverResult::Checked(check) if check.passed() => writeln!(
                    f,
                    "{OK} step, dir, diag, enable, straps, charge pump, temperature"
                )?,
                DriverResult::Checked(check) => {
                    write!(f, "{FAIL}")?;
                    let faults = [
                        (!check.step, "STEP not seen by driver"),
                        (!check.dir, "DIR not seen by driver"),
                        (!check.diag, "DIAG disagrees with driver"),
                        (!check.enable_off, "ENN active while disabled"),
                        (!check.straps, "MS1/MS2 straps do not match address"),
                        (!check.charge_pump, "charge pump undervoltage"),
                        (
                            check.temperature == Temperature::Warning,
                            "overtemperature warning",
                        ),
                        (
                            check.temperature == Temperature::Shutdown,
                            "overtemperature shutdown",
                        ),
                    ];
                    let mut first = true;
                    for (_, fault) in faults.iter().filter(|(failed, _)| *failed) {
                        write!(f, "{} {fault}", if first { "" } else { ";" })?;
                        first = false;
                    }
                    writeln!(f)?;
                }
            }
        }

        let totals = Totals {
            failures: self.failures(),
            warnings: self.warnings(),
        };
        write!(f, "{totals}")
    }
}
