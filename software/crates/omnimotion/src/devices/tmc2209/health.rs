use ::tmc2209::reg;
use serde::Serialize;

/// What a driver last said about itself and its outputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DriverStatus {
    /// It answered; everything below is zero when it did not.
    pub connected: bool,
    pub version: u8,
    pub standstill: bool,
    pub open_load_a: bool,
    pub open_load_b: bool,
    pub short_to_ground_a: bool,
    pub short_to_ground_b: bool,
    pub short_to_supply_a: bool,
    pub short_to_supply_b: bool,
    pub overtemperature_warning: bool,
    pub overtemperature: bool,
}

/// A report that must stop the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverProblem {
    /// No answer. On USB power alone a driver cannot answer, so this only
    /// counts where the motor supply is known to be up.
    Silent,
    Overtemperature,
    /// The driver will shut down soon if it keeps heating.
    OvertemperatureWarning,
    ShortCircuit,
}

impl DriverStatus {
    pub const UNKNOWN: Self = Self {
        connected: false,
        version: 0,
        standstill: false,
        open_load_a: false,
        open_load_b: false,
        short_to_ground_a: false,
        short_to_ground_b: false,
        short_to_supply_a: false,
        short_to_supply_b: false,
        overtemperature_warning: false,
        overtemperature: false,
    };

    pub fn from_registers(version: u8, status: reg::DRV_STATUS) -> Self {
        Self {
            connected: true,
            version,
            standstill: status.stst(),
            open_load_a: status.ola(),
            open_load_b: status.olb(),
            short_to_ground_a: status.s2ga(),
            short_to_ground_b: status.s2gb(),
            short_to_supply_a: status.s2vsa(),
            short_to_supply_b: status.s2vsb(),
            overtemperature_warning: status.otpw(),
            overtemperature: status.ot(),
        }
    }

    pub fn shorted(&self) -> bool {
        self.short_to_ground_a
            || self.short_to_ground_b
            || self.short_to_supply_a
            || self.short_to_supply_b
    }

    /// The worst problem reported. Open loads are left out: the driver can
    /// flag them at high velocity with the coils connected.
    pub fn problem(&self) -> Option<DriverProblem> {
        if !self.connected {
            Some(DriverProblem::Silent)
        } else if self.overtemperature {
            Some(DriverProblem::Overtemperature)
        } else if self.overtemperature_warning {
            Some(DriverProblem::OvertemperatureWarning)
        } else if self.shorted() {
            Some(DriverProblem::ShortCircuit)
        } else {
            None
        }
    }
}
