use ::tmc2209::reg::{self, ReadableRegister, WritableRegister};
use embassy_futures::select::{select, Either};
use embedded_hal_async::delay::DelayNs;
use embedded_io_async::{Read, ReadExactError, Write};

use super::config::ConfigError;
use super::ADDRESSES;

const REPLY_BYTES: usize = 8;
const READ_ATTEMPTS: usize = 3;
const WRITE_ATTEMPTS: usize = 3;

/// A reply takes about 1.2 ms at 115200 baud, so this is generous margin.
const TIMEOUT_MS: u32 = 20;

#[derive(Debug)]
pub enum Error<E> {
    Io(E),
    /// No valid reply after every attempt: absent, unpowered, or a noisy bus.
    NoReply,
    /// A reply arrived carrying a register we did not ask for.
    WrongRegister,
    InvalidEcho,
    InvalidAddress(u8),
    WriteNotConfirmed,
    DriverFault,
    InvalidConfiguration(ConfigError),
}

impl<E> From<ConfigError> for Error<E> {
    fn from(error: ConfigError) -> Self {
        Self::InvalidConfiguration(error)
    }
}

/// The single-wire UART shared by all three drivers. TX and RX sit on the same
/// wire, so every datagram the master sends comes straight back into RX and has
/// to be consumed before a reply can be read. Shadows each driver's write-mostly
/// CHOPCONF so a current or microstep change needs no read-modify-write.
pub struct Bus<S, D> {
    serial: S,
    delay: D,
    chopconf: [Option<reg::CHOPCONF>; ADDRESSES.len()],
}

impl<S: Read + Write, D: DelayNs> Bus<S, D> {
    pub fn new(serial: S, delay: D) -> Self {
        Self {
            serial,
            delay,
            chopconf: [None; ADDRESSES.len()],
        }
    }

    pub fn into_inner(self) -> S {
        self.serial
    }

    pub async fn write<R: WritableRegister>(
        &mut self,
        address: u8,
        register: R,
    ) -> Result<(), Error<S::Error>> {
        self.validate_address(address)?;
        let request = ::tmc2209::write_request(address, register);
        if request.bytes()[2] & 0x7f == reg::Address::CHOPCONF as u8 {
            self.invalidate_chopconf(address);
        }
        for _ in 0..WRITE_ATTEMPTS {
            let before = self.read::<reg::IFCNT>(address).await?.get() as u8;
            match self.send(request.bytes()).await {
                Ok(()) | Err(Error::InvalidEcho) | Err(Error::NoReply) => {}
                Err(error) => return Err(error),
            }

            match self.read::<reg::IFCNT>(address).await {
                Ok(after) if after.get() == before.wrapping_add(1) => return Ok(()),
                Ok(_) | Err(Error::NoReply | Error::InvalidEcho | Error::WrongRegister) => {}
                Err(error) => return Err(error),
            }
        }

        Err(Error::WriteNotConfirmed)
    }

    pub async fn read<R: ReadableRegister>(&mut self, address: u8) -> Result<R, Error<S::Error>> {
        self.validate_address(address)?;
        let mut wrong_register = false;
        for _ in 0..READ_ATTEMPTS {
            let request = ::tmc2209::read_request::<R>(address);
            match self.send(request.bytes()).await {
                Ok(()) => {}
                Err(Error::InvalidEcho) => {
                    let mut discard = [0; REPLY_BYTES];
                    let _ = self.receive(&mut discard).await;
                    continue;
                }
                Err(Error::NoReply) => continue,
                Err(error) => return Err(error),
            }

            let mut reply = [0u8; REPLY_BYTES];
            match self.receive(&mut reply).await {
                Ok(()) => {}
                Err(Error::NoReply) => continue,
                Err(error) => return Err(error),
            }

            if reply[0] != ::tmc2209::SYNC_AND_RESERVED || reply[1] != ::tmc2209::MASTER_ADDR {
                continue;
            }
            let mut reader = ::tmc2209::Reader::default();
            if let (_, Some(response)) = reader.read_response(&reply) {
                if response.crc_is_valid() {
                    match response.register::<R>() {
                        Ok(register) => return Ok(register),
                        Err(_) => wrong_register = true,
                    }
                }
            }
        }

        if wrong_register {
            Err(Error::WrongRegister)
        } else {
            Err(Error::NoReply)
        }
    }

    pub(crate) fn cached_chopconf(&self, address: u8) -> Option<reg::CHOPCONF> {
        self.chopconf.get(address as usize).copied().flatten()
    }

    pub(crate) fn cache_chopconf(&mut self, address: u8, value: reg::CHOPCONF) {
        if let Some(slot) = self.chopconf.get_mut(address as usize) {
            *slot = Some(value);
        }
    }

    fn invalidate_chopconf(&mut self, address: u8) {
        if let Some(slot) = self.chopconf.get_mut(address as usize) {
            *slot = None;
        }
    }

    async fn send(&mut self, bytes: &[u8]) -> Result<(), Error<S::Error>> {
        self.serial.write_all(bytes).await.map_err(Error::Io)?;
        self.serial.flush().await.map_err(Error::Io)?;

        let mut echo = [0u8; REPLY_BYTES];
        self.receive(&mut echo[..bytes.len()]).await?;
        if echo[..bytes.len()] == *bytes {
            Ok(())
        } else {
            Err(Error::InvalidEcho)
        }
    }

    fn validate_address(&self, address: u8) -> Result<(), Error<S::Error>> {
        if ADDRESSES.contains(&address) {
            Ok(())
        } else {
            Err(Error::InvalidAddress(address))
        }
    }

    /// A UART never reports EOF, so without the timeout an absent driver would
    /// leave `read_exact` pending forever.
    async fn receive(&mut self, buffer: &mut [u8]) -> Result<(), Error<S::Error>> {
        let Self { serial, delay, .. } = self;

        match select(serial.read_exact(buffer), delay.delay_ms(TIMEOUT_MS)).await {
            Either::First(result) => result.map_err(read_error),
            Either::Second(()) => Err(Error::NoReply),
        }
    }
}

fn read_error<E>(error: ReadExactError<E>) -> Error<E> {
    match error {
        ReadExactError::UnexpectedEof => Error::NoReply,
        ReadExactError::Other(error) => Error::Io(error),
    }
}
