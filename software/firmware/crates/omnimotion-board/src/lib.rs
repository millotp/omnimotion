#![no_std]

//! Run `FaultSupervisor` continuously; nothing else disables the drivers.

mod board;
mod boot;
pub mod control;
pub mod diag;
pub mod motion;
pub mod net;
pub mod pins;
pub mod storage;

pub use board::{Board, Config, StartError, TmcBus, TmcDriver, TmcError};
pub use boot::{boot, boot_on_second_core, Rtos};
pub use omnimotion::devices::endstop::{EndstopConfig, EndstopKind, EndstopRole, ENDSTOPS};
pub use omnimotion::devices::tmc2209;
pub use omnimotion::devices::tmc2209::{DriverConfig, MicroStepResolution};
pub use omnimotion::{steps_per_mm, AxisConfig, AXES};
pub use pins::BoardPins;
