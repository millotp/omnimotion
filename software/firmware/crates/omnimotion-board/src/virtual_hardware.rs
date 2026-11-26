//! What stands in for the machine when the board has nothing attached (feature `virtual-hardware`).
//!
//! The TMC2209s are `omnimotion::devices::tmc2209::emulator::TmcEmulator`, which reads back the STEP and
//! DIR levels the wiring check drives ([`pins`]), VMOTOR reads [`MOTOR_MV`] and the e-stop is closed. The
//! endstops come from a [`Sensors`] model the application installs: it gets the step counters, which the
//! step engine keeps whether or not a motor is connected, and says which endstops would be triggered with
//! the machine at that position.
//!
//! An operator's hands can be borrowed with [`hand`]: it moves the machine the model sees away from where
//! the step counters put it, and holds endstops triggered, without a step being taken.

extern crate alloc;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicI32, AtomicPtr, AtomicU8, Ordering};

use omnimotion::devices::tmc2209::emulator::StepDir;
use omnimotion::AXES;

use crate::motion::stepper;

/// VMOTOR as a 24 V supply: inside the board's motor-ready window.
pub const MOTOR_MV: i32 = 24_000;

pub trait Sensors: Sync {
    /// The endstops triggered (bit n is En) with each axis at `raw_steps`, the step engine's counters.
    fn triggered(&self, raw_steps: [i64; AXES]) -> u8;
}

/// Points at a leaked `&'static dyn Sensors`, a thin pointer an atomic can hold.
static SENSORS: AtomicPtr<&'static dyn Sensors> = AtomicPtr::new(core::ptr::null_mut());

/// Installs the machine model. With none installed, no endstop is ever triggered.
pub fn install(model: &'static dyn Sensors) {
    SENSORS.store(Box::leak(Box::new(model)), Ordering::Release);
}

/// What a hand has done to the machine: counters added to the step engine's, and endstops held on.
static HAND_STEPS: [AtomicI32; AXES] = [const { AtomicI32::new(0) }; AXES];
static HAND_HELD: AtomicU8 = AtomicU8::new(0);

/// Moves each axis `offsets` steps from where the step engine has it, as a hand on the wheel or the slide
/// would, and holds the endstops in `held` triggered, as a hand on the dancer would. `hand([0; AXES], 0)`
/// lets go.
pub fn hand(offsets: [i32; AXES], held: u8) {
    for (counter, offset) in HAND_STEPS.iter().zip(offsets) {
        counter.store(offset, Ordering::Relaxed);
    }
    HAND_HELD.store(held, Ordering::Relaxed);
}

/// The endstops triggered with the machine where the step engine has put it, and the hand has moved it.
pub fn triggered() -> u8 {
    let mut position = stepper::position();
    for (steps, offset) in position.iter_mut().zip(&HAND_STEPS) {
        *steps += i64::from(offset.load(Ordering::Relaxed));
    }
    let held = HAND_HELD.load(Ordering::Relaxed);
    // SAFETY: only `install` stores here, and it stores a pointer to a leaked box.
    match unsafe { SENSORS.load(Ordering::Acquire).as_ref() } {
        Some(model) => model.triggered(position) | held,
        None => held,
    }
}

/// STEP in bit 0 and DIR in bit 1, as each axis's lines were last driven.
static LINES: [AtomicU8; AXES] = [const { AtomicU8::new(0) }; AXES];

/// Records the levels the wiring check drives on `axis`, for the emulated driver to read back.
pub(crate) fn set_lines(axis: usize, step: bool, dir: bool) {
    LINES[axis].store(u8::from(step) | u8::from(dir) << 1, Ordering::Relaxed);
}

/// The levels on the STEP and DIR pins of the driver at UART address `axis`.
pub fn pins(axis: usize) -> StepDir {
    let lines = LINES
        .get(axis)
        .map_or(0, |lines| lines.load(Ordering::Relaxed));
    StepDir {
        step: lines & 1 != 0,
        dir: lines & 2 != 0,
    }
}
