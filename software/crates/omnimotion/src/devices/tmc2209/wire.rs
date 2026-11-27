//! The driver's side of the single-wire UART, for the stand-ins that play a TMC2209: finding requests in
//! the bytes the MCU sends, building replies, and the register values a driver powers up with.

use ::tmc2209::reg::Address;

/// Bit 7 of a datagram's register byte marks a write.
pub const WRITE_FLAG: u8 = 0x80;
pub const READ_DATAGRAM: usize = 4;
pub const WRITE_DATAGRAM: usize = 8;

/// `GSTAT` has three flags, each cleared by writing a 1 to it.
pub const GSTAT_FLAGS: u32 = 0b111;

/// Power-on values from the datasheet, for the registers that do not read as zero.
pub const POWER_ON: [(Address, u32); 7] = [
    (Address::GCONF, 0x0000_0101),
    // The reset flag.
    (Address::GSTAT, 0x1),
    (Address::IHOLD_IRUN, 0x0000_1F10),
    (Address::TPOWERDOWN, 20),
    (Address::TSTEP, 0x000F_FFFF),
    (Address::CHOPCONF, 0x1000_0053),
    // Standing still.
    (Address::DRV_STATUS, 1 << 31),
];

/// The power-on value of `register`.
pub fn power_on(register: Address) -> u32 {
    POWER_ON
        .iter()
        .find(|(address, _)| *address == register)
        .map_or(0, |(_, value)| *value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Read {
        address: u8,
        register: u8,
    },
    Write {
        address: u8,
        register: u8,
        value: u32,
    },
}

/// Looks for a request at the start of `bytes`. Returns how many bytes can be dropped, with the request
/// they held: leading line noise and datagrams with a bad CRC are dropped with none, and a datagram that
/// has not all arrived yet drops nothing.
pub fn next_request(bytes: &[u8]) -> (usize, Option<Request>) {
    let Some(start) = bytes
        .iter()
        .position(|&byte| byte == ::tmc2209::SYNC_AND_RESERVED)
    else {
        return (bytes.len(), None);
    };
    if start > 0 {
        return (start, None);
    }
    let Some(&register) = bytes.get(2) else {
        return (0, None);
    };
    let length = if register & WRITE_FLAG != 0 {
        WRITE_DATAGRAM
    } else {
        READ_DATAGRAM
    };
    let Some(datagram) = bytes.get(..length) else {
        return (0, None);
    };
    if ::tmc2209::crc(&datagram[..length - 1]) != datagram[length - 1] {
        return (length, None);
    }
    let (address, register) = (datagram[1], register & !WRITE_FLAG);
    let request = if length == WRITE_DATAGRAM {
        let value = u32::from_be_bytes([datagram[3], datagram[4], datagram[5], datagram[6]]);
        Request::Write {
            address,
            register,
            value,
        }
    } else {
        Request::Read { address, register }
    };
    (length, Some(request))
}

/// The reply to a read of `register`.
pub fn reply(register: u8, value: u32) -> [u8; WRITE_DATAGRAM] {
    let mut reply = [0; WRITE_DATAGRAM];
    reply[0] = ::tmc2209::SYNC_AND_RESERVED;
    reply[1] = ::tmc2209::MASTER_ADDR;
    reply[2] = register;
    reply[3..7].copy_from_slice(&value.to_be_bytes());
    reply[7] = ::tmc2209::crc(&reply[..7]);
    reply
}

/// The levels on the pins a driver reports in `IOIN`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pins {
    /// ENN is high: the outputs are off.
    pub disabled: bool,
    pub diag: bool,
    pub step: bool,
    pub dir: bool,
}

pub const IOIN_ENN: u32 = 1 << 0;
pub const IOIN_MS1: u32 = 1 << 2;
pub const IOIN_MS2: u32 = 1 << 3;
pub const IOIN_DIAG: u32 = 1 << 4;
pub const IOIN_PDN_UART: u32 = 1 << 6;
pub const IOIN_STEP: u32 = 1 << 7;
pub const IOIN_DIR: u32 = 1 << 9;

/// `IOIN` of a TMC2209 strapped to `address` on the UART, with `pins`.
pub fn ioin(address: u8, pins: Pins) -> u32 {
    let bit = |set: bool, mask: u32| if set { mask } else { 0 };
    u32::from(super::VERSION) << 24
        | IOIN_PDN_UART
        | bit(pins.disabled, IOIN_ENN)
        | bit(address & 1 != 0, IOIN_MS1)
        | bit(address & 2 != 0, IOIN_MS2)
        | bit(pins.diag, IOIN_DIAG)
        | bit(pins.step, IOIN_STEP)
        | bit(pins.dir, IOIN_DIR)
}
