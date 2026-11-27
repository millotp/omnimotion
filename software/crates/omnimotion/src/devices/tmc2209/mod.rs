pub mod bus;
pub mod config;
pub mod emulator;
pub mod health;
pub mod stallguard;
pub mod wire;

pub use ::tmc2209::data::MicroStepResolution;
pub use ::tmc2209::reg;
pub use bus::{Bus, Error};
pub use config::{
    current_scale, current_scales, microstep_resolution, ConfigError, Driver, DriverConfig,
    MAX_CURRENT_MA, R_SENSE, VERSION,
};
pub use health::{DriverProblem, DriverStatus};
pub use stallguard::tstep_for_rate;

/// UART addresses of the three drivers, set by the MS1/MS2 straps.
pub const ADDRESSES: [u8; 3] = [0, 1, 2];

/// The TMC2209 allows 500 kbit/s; 115200 leaves margin on a bus shared by three.
pub const BAUD: u32 = 115_200;
