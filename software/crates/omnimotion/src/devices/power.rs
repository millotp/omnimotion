use core::fmt;

/// R2 100 k / R3 10 k divider on VMOTOR.
const MOTOR_RATIO: i32 = 11;

const MOTOR_PRESENT_MV: i32 = 1_000;

/// 12-24 V board. The ceiling leaves headroom below the TMC2209's 32 V limit.
const MOTOR_MIN_MV: i32 = 11_000;
const MOTOR_MAX_MV: i32 = 26_000;

/// Whether the board is running off bulk supply or USB alone. Inferred from
/// VMOTOR because the TPS2116 gives the buck priority, so a live VMOTOR means
/// the buck owns 5V_SYS. The mux's own ST pin only reaches a test point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerSource {
    /// USB, so the whole board has a 500 mA budget: no motors, no outputs.
    UsbOnly,
    Bulk,
    BulkOutOfRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rails {
    pub motor_mv: i32,
}

impl Rails {
    pub fn from_pin_mv(pin_mv: i32) -> Self {
        Self {
            motor_mv: (pin_mv * MOTOR_RATIO).max(0),
        }
    }

    pub fn source(self) -> PowerSource {
        if self.motor_mv < MOTOR_PRESENT_MV {
            PowerSource::UsbOnly
        } else if (MOTOR_MIN_MV..=MOTOR_MAX_MV).contains(&self.motor_mv) {
            PowerSource::Bulk
        } else {
            PowerSource::BulkOutOfRange
        }
    }
}

pub struct Volts(pub i32);

impl fmt::Display for Volts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:02} V", self.0 / 1_000, self.0 % 1_000 / 10)
    }
}
