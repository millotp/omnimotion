pub mod bus;
pub mod config;
pub mod stallguard;

pub use ::tmc2209::data::MicroStepResolution;
pub use ::tmc2209::reg;
pub use bus::{Bus, Error};
pub use config::{
    current_scale, current_scales, microstep_resolution, ConfigError, Driver, DriverConfig,
    MAX_CURRENT_MA, R_SENSE, VERSION,
};
pub use stallguard::tstep_for_rate;

/// UART addresses of the three drivers, set by the MS1/MS2 straps.
pub const ADDRESSES: [u8; 3] = [0, 1, 2];
