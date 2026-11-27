//! The board's three TMC2209 sockets (M0 to M2, each fitted or not) on its single-wire UART: register file,
//! microstep sequencer, current regulation and the datagram protocol with its echo and timing.

use std::collections::VecDeque;
use std::f64::consts::PI;
use std::task::{Poll, Waker};

use omnimotion::devices::tmc2209::reg::{self, Address};
pub use omnimotion::devices::tmc2209::wire::Pins;
use omnimotion::devices::tmc2209::wire::{self, Request, GSTAT_FLAGS};
pub use omnimotion::devices::tmc2209::BAUD;
use omnimotion::devices::tmc2209::R_SENSE;
use serde::{Deserialize, Serialize};
use tmc2209::INTERNAL_CLOCK_HZ;

use crate::clock;

const BIT_NS: f64 = 1e9 / BAUD as f64;
const BYTE_NS: u64 = (10.0 * BIT_NS) as u64;
/// Standstill is flagged 2^20 clocks after the last step.
const STANDSTILL_US: u64 = ((1u32 << 20) as f32 / INTERNAL_CLOCK_HZ * 1e6) as u64;
/// TPOWERDOWN counts in 2^18 clocks.
const POWERDOWN_UNIT_US: u64 = ((1u32 << 18) as f32 / INTERNAL_CLOCK_HZ * 1e6) as u64;
/// i_scale_analog = 1 (the reset value) scales the sense voltage by VREF / 2.5 V; VREF comes from the
/// board's 33k/10k divider on 3.3 V.
const ANALOG_SCALE: f64 = 0.767 / 2.5;
/// The ESP32-S3 UART's receive FIFO, bytes.
const RX_FIFO: usize = 128;

#[derive(Clone, Serialize, Deserialize)]
pub struct Chip {
    pub fitted: bool,
    pub powered: bool,
    pub address: u8,
    gconf: u32,
    gstat: u32,
    ifcnt: u8,
    slaveconf: u32,
    ihold_irun: u32,
    tpowerdown: u32,
    chopconf: u32,
    /// Sequencer position in 1/256 full steps, unwrapped.
    pub counts: i64,
    /// Power-on resets so far: each one puts the sequencer back to its start.
    pub power_ons: u32,
    last_step: Option<u64>,
    step_interval: u64,
    powered_at: u64,
}

impl Chip {
    fn new(address: u8, fitted: bool) -> Self {
        let mut chip = Self {
            fitted,
            powered: false,
            address,
            gconf: 0,
            gstat: 0,
            ifcnt: 0,
            slaveconf: 0,
            ihold_irun: 0,
            tpowerdown: 0,
            chopconf: 0,
            counts: 0,
            power_ons: 0,
            last_step: None,
            step_interval: 0,
            powered_at: 0,
        };
        chip.reset(0);
        chip
    }

    fn reset(&mut self, now: u64) {
        self.gconf = wire::power_on(Address::GCONF);
        self.gstat = wire::power_on(Address::GSTAT);
        self.ifcnt = 0;
        self.slaveconf = wire::power_on(Address::SLAVECONF);
        self.ihold_irun = wire::power_on(Address::IHOLD_IRUN);
        self.tpowerdown = wire::power_on(Address::TPOWERDOWN);
        self.chopconf = wire::power_on(Address::CHOPCONF);
        self.counts = 0;
        self.power_ons += 1;
        self.last_step = None;
        self.powered_at = now;
    }

    pub fn shaft(&self) -> bool {
        reg::GCONF(self.gconf).shaft()
    }

    /// Microsteps per full step: CHOPCONF.MRES, or the MS1/MS2 address straps until mstep_reg_select.
    pub fn microsteps(&self) -> u32 {
        if reg::GCONF(self.gconf).mstep_reg_select() {
            256 >> ((self.chopconf >> 24) & 0xF).min(8)
        } else {
            match self.address & 3 {
                0 => 8,
                1 => 32,
                2 => 64,
                _ => 16,
            }
        }
    }

    fn standstill(&self, now: u64) -> bool {
        match self.last_step {
            None => true,
            Some(t) => now.saturating_sub(t) >= STANDSTILL_US,
        }
    }

    fn current_scale(&self, now: u64) -> u8 {
        let currents = reg::IHOLD_IRUN(self.ihold_irun);
        let (ihold, irun) = (currents.ihold(), currents.irun());
        let since = self.last_step.unwrap_or(self.powered_at);
        let idle = now.saturating_sub(since);
        if self.standstill(now)
            && idle >= STANDSTILL_US + u64::from(self.tpowerdown) * POWERDOWN_UNIT_US
        {
            ihold
        } else {
            irun
        }
    }

    /// Phase current the chopper regulates to, A rms; zero when disabled or unpowered.
    pub fn current_rms(&self, enabled: bool, now: u64) -> f64 {
        if !self.fitted || !self.powered || !enabled {
            return 0.0;
        }
        let analog = if reg::GCONF(self.gconf).i_scale_analog() {
            ANALOG_SCALE
        } else {
            1.0
        };
        let vsense = reg::CHOPCONF(self.chopconf).vsense();
        let ma = tmc2209::vsense_cs_to_rms_current(R_SENSE, vsense, self.current_scale(now));
        f64::from(ma) / 1000.0 * analog
    }

    /// Electrical angle the coil currents point at, radians (positive turns the shaft positive).
    pub fn electrical_angle(&self) -> f64 {
        let angle = self.counts as f64 * (2.0 * PI / 1024.0);
        if self.shaft() {
            -angle
        } else {
            angle
        }
    }

    fn step(&mut self, forward: bool, now: u64) {
        if !self.fitted || !self.powered {
            return;
        }
        let inc = i64::from(256 / self.microsteps());
        self.counts += if forward { inc } else { -inc };
        if let Some(t) = self.last_step {
            self.step_interval = now - t;
        }
        self.last_step = Some(now);
    }

    fn read(&self, register: u8, pins: Pins, now: u64) -> u32 {
        let Ok(register) = Address::try_from(register) else {
            return 0;
        };
        match register {
            Address::GCONF => self.gconf,
            Address::GSTAT => self.gstat,
            Address::IFCNT => u32::from(self.ifcnt),
            Address::IOIN => wire::ioin(self.address, pins),
            Address::TSTEP => {
                if self.standstill(now) {
                    wire::power_on(Address::TSTEP)
                } else {
                    let clocks_per_us = f64::from(INTERNAL_CLOCK_HZ) / 1e6;
                    let per_256 = self.step_interval as f64 * clocks_per_us
                        / f64::from(256 / self.microsteps());
                    (per_256 as u32).min(wire::power_on(Address::TSTEP))
                }
            }
            Address::SG_RESULT => {
                if self.standstill(now) {
                    0
                } else {
                    300
                }
            }
            Address::CHOPCONF => self.chopconf,
            Address::DRV_STATUS => {
                let stealth = !reg::GCONF(self.gconf).en_spread_cycle();
                u32::from(self.standstill(now)) << 31
                    | u32::from(stealth) << 30
                    | u32::from(self.current_scale(now)) << 16
            }
            _ => 0,
        }
    }

    /// Registers the model does not use (TPWMTHRS, TCOOLTHRS, SGTHRS, COOLCONF, PWMCONF) are accepted and
    /// counted, but not kept: nothing reads them back.
    fn write(&mut self, register: u8, value: u32) {
        match Address::try_from(register) {
            Ok(Address::GCONF) => self.gconf = value & 0x3FF,
            Ok(Address::GSTAT) => self.gstat &= !(value & GSTAT_FLAGS),
            Ok(Address::SLAVECONF) => self.slaveconf = value & 0xF00,
            Ok(Address::IHOLD_IRUN) => self.ihold_irun = value & 0x000F_1F1F,
            Ok(Address::TPOWERDOWN) => self.tpowerdown = value & 0xFF,
            Ok(Address::CHOPCONF) => self.chopconf = value,
            _ => {}
        }
        self.ifcnt = self.ifcnt.wrapping_add(1);
    }

    /// Bit times between the end of a read request and the reply (SLAVECONF.SENDDELAY).
    fn send_delay_bits(&self) -> f64 {
        let n = (self.slaveconf >> 8) & 0xF;
        8.0 * f64::from(2 * (n / 2) + 1)
    }
}

/// More bytes arrived than the receive FIFO holds before the MCU read them; the FIFO was reset.
#[derive(Debug)]
pub struct Overrun;

/// What the MCU's UART receives: its own bytes echoed back, then any reply, each with its arrival time.
#[derive(Default)]
struct Line {
    tx_done_ns: u64,
    busy_ns: u64,
    rx: VecDeque<(u64, u8)>,
    request: Vec<u8>,
    waker: Option<Waker>,
}

#[derive(Serialize, Deserialize)]
pub struct Drivers {
    pub chips: [Chip; 3],
    /// DRV_EN: ENN of all three drivers, inverted.
    pub enabled: bool,
    #[serde(skip)]
    line: Line,
}

impl Drivers {
    /// Drivers in the sockets marked `fitted`, unpowered.
    pub fn new(fitted: [bool; 3]) -> Self {
        Self {
            chips: std::array::from_fn(|axis| Chip::new(axis as u8, fitted[axis])),
            enabled: false,
            line: Line::default(),
        }
    }

    pub fn set_power(&mut self, on: bool, now: u64) {
        for chip in &mut self.chips {
            if on && !chip.powered {
                chip.reset(now);
            }
            chip.powered = on && chip.fitted;
        }
    }

    pub fn step(&mut self, axis: usize, forward: bool, now: u64) {
        self.chips[axis].step(forward, now);
    }

    /// Bytes written by the MCU at `now`; `pins` gives the pin levels each driver sees.
    pub fn transmit(&mut self, bytes: &[u8], now: u64, pins: impl Fn(usize) -> Pins) {
        let line = &mut self.line;
        for &byte in bytes {
            let start = (now * 1000).max(line.busy_ns);
            let end = start + BYTE_NS;
            line.busy_ns = end;
            line.tx_done_ns = end;
            line.rx.push_back((end.div_ceil(1000), byte));
            line.request.push(byte);
            let (used, request) = wire::next_request(&line.request);
            line.request.drain(..used);
            let Some(
                request @ (Request::Read { address, register }
                | Request::Write {
                    address, register, ..
                }),
            ) = request
            else {
                continue;
            };
            let Some(index) = self
                .chips
                .iter()
                .position(|c| c.address == address && c.fitted && c.powered)
            else {
                continue;
            };
            let chip = &mut self.chips[index];
            match request {
                Request::Write { value, .. } => chip.write(register, value),
                Request::Read { .. } => {
                    let value = chip.read(register, pins(index), now);
                    let mut t = end + (chip.send_delay_bits() * BIT_NS) as u64;
                    for b in wire::reply(register, value) {
                        t = t.max(line.busy_ns) + BYTE_NS;
                        line.busy_ns = t;
                        line.rx.push_back((t.div_ceil(1000), b));
                    }
                }
            }
        }
        if let Some(waker) = line.waker.take() {
            waker.wake();
        }
    }

    /// Copies received bytes that have arrived by `now`; pending (with a wake-up booked) if none have.
    pub fn receive(
        &mut self,
        buf: &mut [u8],
        now: u64,
        waker: &Waker,
    ) -> Poll<Result<usize, Overrun>> {
        let line = &mut self.line;
        let arrived = line.rx.iter().take_while(|&&(at, _)| at <= now).count();
        if arrived > RX_FIFO {
            line.rx.drain(..arrived);
            return Poll::Ready(Err(Overrun));
        }
        if arrived > 0 {
            let n = arrived.min(buf.len());
            for (slot, (_, byte)) in buf.iter_mut().zip(line.rx.drain(..n)) {
                *slot = byte;
            }
            return Poll::Ready(Ok(n));
        }
        if let Some(&(at, _)) = line.rx.front() {
            clock::wake_at(at, waker);
        }
        line.waker = Some(waker.clone());
        Poll::Pending
    }

    /// Whether the MCU's own bytes have left the shift register by `now`; books a wake-up if not.
    pub fn flushed(&mut self, now: u64, waker: &Waker) -> bool {
        let done = self.line.tx_done_ns.div_ceil(1000);
        if done <= now {
            true
        } else {
            clock::wake_at(done, waker);
            false
        }
    }

    pub fn currents(&self, now: u64) -> [f64; 3] {
        std::array::from_fn(|i| self.chips[i].current_rms(self.enabled, now))
    }
}

impl Default for Drivers {
    fn default() -> Self {
        Self::new([false; 3])
    }
}
