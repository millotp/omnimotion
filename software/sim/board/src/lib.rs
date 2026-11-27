#![no_std]

//! omnimotion-board for the simulator: the production modules below are compiled from the Omnimotion
//! sources as they are; `boot`, `net` and `storage` (apart from its flash lock and persistence layer) are
//! the simulated stand-ins, with the same API.

extern crate alloc;
extern crate std;

// The shimmed peripherals are unit structs, which the production code builds with `Default::default()`.
#[allow(clippy::default_constructed_unit_structs)]
#[path = "../../../firmware/crates/omnimotion-board/src/board.rs"]
mod board;
mod boot;
#[path = "../../../firmware/crates/omnimotion-board/src/control.rs"]
pub mod control;
#[path = "../../../firmware/crates/omnimotion-board/src/diag/mod.rs"]
pub mod diag;
// The shimmed timer interrupt takes no priority, so the production `Priority` import goes unused.
#[allow(unused_imports, clippy::default_constructed_unit_structs)]
#[path = "../../../firmware/crates/omnimotion-board/src/motion/mod.rs"]
pub mod motion;
pub mod net;
#[path = "../../../firmware/crates/omnimotion-board/src/pins.rs"]
pub mod pins;
pub mod storage;

// `motion::homing` and `diag::inventory` name the bus types through the crate root.
pub use board::{Board, Config, StartError, TmcBus, TmcDriver, TmcError};
pub use boot::{boot_on_second_core, Rtos};
pub use omnimotion::devices::endstop::{EndstopConfig, EndstopKind, EndstopRole, ENDSTOPS};
pub use omnimotion::devices::tmc2209;
pub use omnimotion::devices::tmc2209::{DriverConfig, MicroStepResolution};
pub use omnimotion::{AxisConfig, AXES};
pub use pins::BoardPins;
