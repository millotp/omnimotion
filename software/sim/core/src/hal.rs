//! The firmware's view of the board and its machine, called by the stand-in crates.

use std::task::{Poll, Waker};

use omnimotion::devices::power::MOTOR_RATIO;

use crate::hw::{self, Pull, DIAG, DRV_EN, ENDSTOPS, ESTOP};
use crate::{clock, log, tmc, with, World};

pub fn gpio_out() -> u64 {
    with(|w| w.hw.out)
}

pub fn gpio_update(set: u64, clear: u64) {
    let now = clock::now();
    with(|w| {
        let old = w.hw.out;
        let new = (old | set) & !clear;
        w.hw.out = new;
        let rising = new & !old;
        for axis in 0..hw::STEP.len() {
            if rising & (1 << hw::STEP[axis]) != 0 {
                let forward = new & (1 << hw::DIR[axis]) != 0;
                w.drivers.step(axis, forward, now);
                w.plant.wake(now);
            }
        }
        if (old ^ new) & (1 << DRV_EN) != 0 {
            w.drivers.enabled = new & (1 << DRV_EN) != 0;
            w.plant.wake(now);
        }
    });
}

pub fn configure_input(pin: u8, pull: Pull) {
    with(|w| w.hw.pull[pin as usize] = pull);
}

/// What an input pin reads: the e-stop, the drivers' DIAG outputs (low: no StallGuard), the sensors on the
/// endstop connectors, and otherwise its pull (or, with none, the output latch read back).
pub fn level(w: &World, pin: u8, now: u64) -> bool {
    if pin == ESTOP {
        return !w.env.estop_closed;
    }
    if DIAG.contains(&pin) {
        return false;
    }
    let driven = ENDSTOPS
        .iter()
        .position(|&p| p == pin)
        .and_then(|e| w.plant.endstop_level(e, now));
    driven.unwrap_or_else(|| match w.hw.pull[pin as usize] {
        Pull::Up => true,
        Pull::Down => false,
        Pull::None => w.hw.level(pin),
    })
}

pub fn input(pin: u8) -> bool {
    let now = clock::now();
    with(|w| level(w, pin, now))
}

/// VMON pin, mV.
pub fn vmon_mv() -> u16 {
    with(|w| {
        if w.env.supply_on {
            (w.board.supply_v * 1000.0 / f64::from(MOTOR_RATIO)) as u16
        } else {
            0
        }
    })
}

pub fn uart_write(bytes: &[u8]) {
    let now = clock::now();
    with(|w| {
        let out = w.hw.out;
        let enabled = w.drivers.enabled;
        w.drivers.transmit(bytes, now, |axis| tmc::Pins {
            disabled: !enabled,
            diag: false,
            step: out & (1 << hw::STEP[axis]) != 0,
            dir: out & (1 << hw::DIR[axis]) != 0,
        });
    });
}

pub fn uart_read(buf: &mut [u8], waker: &Waker) -> Poll<Result<usize, tmc::Overrun>> {
    let now = clock::now();
    with(|w| w.drivers.receive(buf, now, waker))
}

pub fn uart_flushed(waker: &Waker) -> bool {
    let now = clock::now();
    with(|w| w.drivers.flushed(now, waker))
}

pub fn set_isr(handler: fn()) {
    with(|w| w.hw.isr = Some(handler));
}

pub fn timer<R>(f: impl FnOnce(&mut hw::Timer0, u64) -> R) -> R {
    let now = clock::now();
    with(|w| f(&mut w.hw.timer, now))
}

pub fn watchdog<R>(f: impl FnOnce(&mut hw::Wdt, u64) -> R) -> R {
    let now = clock::now();
    with(|w| f(&mut w.hw.wdt, now))
}

pub fn nvs_get(namespace: &str, key: &str) -> Option<Vec<u8>> {
    with(|w| w.nvs.get(&format!("{namespace}/{key}")).cloned())
}

pub fn nvs_set(namespace: &str, key: &str, value: &[u8]) {
    with(|w| w.nvs.insert(format!("{namespace}/{key}"), value.to_vec()));
}

pub fn console(text: &str) {
    log::line(clock::now(), text);
}
