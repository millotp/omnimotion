//! A stand-in for the TMC2209s on the board's single-wire UART, for running the firmware on a board with
//! nothing attached. It speaks the datagram protocol (every byte sent comes straight back, a read request
//! gets its reply behind the echo) and keeps each driver's registers, so `Driver::apply` and the status
//! reads behave as they do on a healthy driver: the version is 0x21, nothing is faulted, the motor stands
//! still, and a power-on reset is reported until it is acknowledged.

use core::convert::Infallible;

use ::tmc2209::reg::Address;
use embedded_io_async::{ErrorType, Read, Write};

use super::{ADDRESSES, VERSION};

/// Two echoes and a reply fit with room to spare; the bus reads them back at once.
const RX_CAPACITY: usize = 64;
const REGISTERS: usize = 128;
/// Registers carry 7-bit addresses; bit 7 of the address byte marks a write.
const WRITE_FLAG: u8 = 0x80;
const READ_DATAGRAM: usize = 4;
const WRITE_DATAGRAM: usize = 8;
/// `GSTAT` has three flags, each cleared by writing a 1 to it.
const GSTAT_FLAGS: u32 = 0b111;
const GSTAT_RESET: u32 = 1;
/// `DRV_STATUS.STST`: the motor stands still.
const STANDSTILL: u32 = 1 << 31;
/// `IOIN` bits for the pins the driver reads: ENN (high is disabled), MS1, MS2, STEP and DIR.
const IOIN_ENN: u32 = 1 << 0;
const IOIN_MS1: u32 = 1 << 2;
const IOIN_MS2: u32 = 1 << 3;
const IOIN_STEP: u32 = 1 << 7;
const IOIN_DIR: u32 = 1 << 9;

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
        let mut chip = [0; REGISTERS];
        // Power-on values from the datasheet.
        chip[Address::GCONF as usize] = 0x0000_0101;
        chip[Address::GSTAT as usize] = GSTAT_RESET;
        chip[Address::IHOLD_IRUN as usize] = 0x0000_1F10;
        chip[Address::TPOWERDOWN as usize] = 20;
        chip[Address::TSTEP as usize] = 0x000F_FFFF;
        chip[Address::CHOPCONF as usize] = 0x1000_0053;
        chip[Address::DRV_STATUS as usize] = STANDSTILL;
        Self {
            fitted,
            pins: |_| StepDir::default(),
            registers: [chip; ADDRESSES.len()],
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

    /// What the driver at `chip` reads on its pins: ENN inactive (the outputs are off), its address on
    /// MS1 and MS2, DIAG low, and the STEP and DIR levels, over its version.
    fn inputs(&self, chip: usize) -> u32 {
        let StepDir { step, dir } = (self.pins)(chip);
        let address = u32::from(ADDRESSES[chip]);
        u32::from(VERSION) << 24
            | IOIN_ENN
            | if address & 1 != 0 { IOIN_MS1 } else { 0 }
            | if address & 2 != 0 { IOIN_MS2 } else { 0 }
            | if step { IOIN_STEP } else { 0 }
            | if dir { IOIN_DIR } else { 0 }
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
        while bytes.len() >= READ_DATAGRAM {
            if bytes[0] != ::tmc2209::SYNC_AND_RESERVED {
                bytes = &bytes[1..];
                continue;
            }
            let length = if bytes[2] & WRITE_FLAG != 0 {
                WRITE_DATAGRAM
            } else {
                READ_DATAGRAM
            };
            if bytes.len() < length {
                return;
            }
            let (datagram, rest) = bytes.split_at(length);
            if ::tmc2209::crc(&datagram[..length - 1]) == datagram[length - 1] {
                self.handle(datagram);
            }
            bytes = rest;
        }
    }

    fn handle(&mut self, datagram: &[u8]) {
        let Some(chip) = ADDRESSES
            .iter()
            .position(|&address| address == datagram[1])
            .filter(|&chip| self.fitted[chip])
        else {
            return;
        };
        let register = usize::from(datagram[2] & !WRITE_FLAG);
        if datagram.len() == WRITE_DATAGRAM {
            let value = u32::from_be_bytes([datagram[3], datagram[4], datagram[5], datagram[6]]);
            let registers = &mut self.registers[chip];
            if register == Address::GSTAT as usize {
                registers[register] &= !(value & GSTAT_FLAGS);
            } else if register != Address::IOIN as usize && register != Address::DRV_STATUS as usize
            {
                registers[register] = value;
            }
            let count = &mut registers[Address::IFCNT as usize];
            *count = u32::from((*count as u8).wrapping_add(1));
        } else {
            let mut reply = [0u8; WRITE_DATAGRAM];
            reply[0] = ::tmc2209::SYNC_AND_RESERVED;
            reply[1] = ::tmc2209::MASTER_ADDR;
            reply[2] = register as u8;
            let value = if register == Address::IOIN as usize {
                self.inputs(chip)
            } else {
                self.registers[chip][register]
            };
            reply[3..7].copy_from_slice(&value.to_be_bytes());
            reply[7] = ::tmc2209::crc(&reply[..7]);
            self.push(&reply);
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
