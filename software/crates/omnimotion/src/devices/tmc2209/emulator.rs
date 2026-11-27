//! A stand-in for the TMC2209s on the board's single-wire UART, for running the firmware on a board with
//! nothing attached. It speaks the datagram protocol (every byte sent comes straight back, a read request
//! gets its reply behind the echo) and keeps each driver's registers, so `Driver::apply` and the status
//! reads behave as they do on a healthy driver: the version is 0x21, nothing is faulted, the motor stands
//! still, and a power-on reset is reported until it is acknowledged.

use core::convert::Infallible;

use ::tmc2209::reg::Address;
use embedded_io_async::{ErrorType, Read, Write};

use super::wire::{self, Pins, Request, GSTAT_FLAGS};
use super::ADDRESSES;

/// Two echoes and a reply fit with room to spare; the bus reads them back at once.
const RX_CAPACITY: usize = 64;
/// The registers a driver keeps, by address. The emulator lives inside the board, which firmware moves
/// around on a small stack, so it stores these few words and not all 128 addresses: anything else
/// reads as zero and ignores writes.
const KNOWN: [Address; 24] = [
    Address::GCONF,
    Address::GSTAT,
    Address::IFCNT,
    Address::SLAVECONF,
    Address::OTP_PROG,
    Address::OTP_READ,
    Address::IOIN,
    Address::FACTORY_CONF,
    Address::IHOLD_IRUN,
    Address::TPOWERDOWN,
    Address::TSTEP,
    Address::TPWMTHRS,
    Address::TCOOLTHRS,
    Address::VACTUAL,
    Address::SGTHRS,
    Address::SG_RESULT,
    Address::COOLCONF,
    Address::MSCNT,
    Address::MSCURACT,
    Address::CHOPCONF,
    Address::DRV_STATUS,
    Address::PWMCONF,
    Address::PWM_SCALE,
    Address::PWM_AUTO,
];
const REGISTERS: usize = KNOWN.len();

/// Where the register at the datagram's 7-bit `address` is kept, if the emulator keeps it.
fn slot_of(address: u8) -> Option<usize> {
    KNOWN.iter().position(|&known| known as u8 == address)
}

/// The levels of one driver's STEP and DIR inputs, as the board drives them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepDir {
    pub step: bool,
    pub dir: bool,
}

pub struct TmcEmulator {
    /// Which of the three sockets hold a driver; the others never answer.
    fitted: [bool; ADDRESSES.len()],
    /// The levels on the STEP and DIR pins of the driver at each UART address, read whenever `IOIN` is.
    pins: fn(usize) -> StepDir,
    registers: [[u32; REGISTERS]; ADDRESSES.len()],
    rx: [u8; RX_CAPACITY],
    rx_start: usize,
    rx_len: usize,
}

impl TmcEmulator {
    /// `fitted[n]` is whether the driver at UART address `n` is there.
    pub fn new(fitted: [bool; ADDRESSES.len()]) -> Self {
        Self {
            fitted,
            pins: |_| StepDir::default(),
            registers: [KNOWN.map(wire::power_on); ADDRESSES.len()],
            rx: [0; RX_CAPACITY],
            rx_start: 0,
            rx_len: 0,
        }
    }

    /// Reads the STEP and DIR levels from `pins(address)` instead of holding them low, so a wiring check
    /// that drives the lines and reads them back through `IOIN` sees them arrive.
    pub fn with_pins(mut self, pins: fn(usize) -> StepDir) -> Self {
        self.pins = pins;
        self
    }

    fn push(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.rx_len == RX_CAPACITY {
                return;
            }
            self.rx[(self.rx_start + self.rx_len) % RX_CAPACITY] = byte;
            self.rx_len += 1;
        }
    }

    /// Acts on every well-formed request in `bytes`, skipping line noise.
    fn receive_requests(&mut self, mut bytes: &[u8]) {
        loop {
            let (used, request) = wire::next_request(bytes);
            if used == 0 {
                return;
            }
            bytes = &bytes[used..];
            if let Some(request) = request {
                self.handle(request);
            }
        }
    }

    fn handle(&mut self, request: Request) {
        let (Request::Read { address, register }
        | Request::Write {
            address, register, ..
        }) = request;
        let Some(chip) = ADDRESSES
            .iter()
            .position(|&known| known == address)
            .filter(|&chip| self.fitted[chip])
        else {
            return;
        };
        let kept = slot_of(register);
        match request {
            Request::Write { value, .. } => {
                let registers = &mut self.registers[chip];
                if let Some(kept) = kept {
                    if register == Address::GSTAT as u8 {
                        registers[kept] &= !(value & GSTAT_FLAGS);
                    } else if register != Address::IOIN as u8
                        && register != Address::DRV_STATUS as u8
                    {
                        registers[kept] = value;
                    }
                }
                let count = &mut registers[slot_of(Address::IFCNT as u8).expect("IFCNT is kept")];
                *count = u32::from((*count as u8).wrapping_add(1));
            }
            Request::Read { .. } => {
                // ENN reads inactive: the outputs are off whenever the emulator is asked.
                let value = if register == Address::IOIN as u8 {
                    let StepDir { step, dir } = (self.pins)(chip);
                    let pins = Pins {
                        disabled: true,
                        diag: false,
                        step,
                        dir,
                    };
                    wire::ioin(ADDRESSES[chip], pins)
                } else {
                    kept.map_or(0, |kept| self.registers[chip][kept])
                };
                self.push(&wire::reply(register, value));
            }
        }
    }
}

impl ErrorType for TmcEmulator {
    type Error = Infallible;
}

impl Read for TmcEmulator {
    /// Stays pending while nothing has arrived, as a real UART does, so the bus's timeout decides.
    async fn read(&mut self, buffer: &mut [u8]) -> Result<usize, Infallible> {
        if self.rx_len == 0 {
            return core::future::pending().await;
        }
        let count = buffer.len().min(self.rx_len);
        for slot in buffer.iter_mut().take(count) {
            *slot = self.rx[self.rx_start];
            self.rx_start = (self.rx_start + 1) % RX_CAPACITY;
        }
        self.rx_len -= count;
        Ok(count)
    }
}

impl Write for TmcEmulator {
    async fn write(&mut self, buffer: &[u8]) -> Result<usize, Infallible> {
        self.push(buffer);
        self.receive_requests(buffer);
        Ok(buffer.len())
    }

    async fn flush(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}
