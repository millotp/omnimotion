use core::future::poll_fn;
use core::marker::PhantomData;
use core::task::Poll;

use crate::gpio::AnyPin;
use crate::peripherals::UART1;

#[derive(Clone, Copy, Default)]
pub struct Config;

impl Config {
    pub fn with_baudrate(self, baud: u32) -> Self {
        assert_eq!(
            baud,
            omnimotion_sim::tmc::BAUD,
            "the simulated TMC bus runs at the drivers' baud rate"
        );
        self
    }
}

#[derive(Debug)]
pub struct ConfigError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoError {
    /// The receive FIFO overflowed before it was read.
    Overrun,
}

impl core::fmt::Display for IoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl core::error::Error for IoError {}

impl embedded_io_async::Error for IoError {
    fn kind(&self) -> embedded_io_async::ErrorKind {
        embedded_io_async::ErrorKind::Other
    }
}

/// UART1 on the TMC2209s' single wire (TX GPIO16, RX GPIO47).
pub struct Uart<'d, Dm> {
    _p: PhantomData<(&'d mut (), Dm)>,
}

impl<'d> Uart<'d, crate::Blocking> {
    pub fn new(_uart: UART1<'d>, _config: Config) -> Result<Self, ConfigError> {
        Ok(Self { _p: PhantomData })
    }

    pub fn into_async(self) -> Uart<'d, crate::Async> {
        Uart { _p: PhantomData }
    }
}

impl<'d, Dm> Uart<'d, Dm> {
    pub fn with_tx(self, _pin: impl Into<AnyPin<'d>>) -> Self {
        self
    }
    pub fn with_rx(self, _pin: impl Into<AnyPin<'d>>) -> Self {
        self
    }
}

impl<Dm> embedded_io_async::ErrorType for Uart<'_, Dm> {
    type Error = IoError;
}

impl embedded_io_async::Read for Uart<'_, crate::Async> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        if buf.is_empty() {
            return Ok(0);
        }
        poll_fn(|cx| omnimotion_sim::hal::uart_read(buf, cx.waker()).map_err(|_| IoError::Overrun))
            .await
    }
}

impl embedded_io_async::Write for Uart<'_, crate::Async> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, IoError> {
        omnimotion_sim::hal::uart_write(buf);
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), IoError> {
        poll_fn(|cx| {
            if omnimotion_sim::hal::uart_flushed(cx.waker()) {
                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            }
        })
        .await
    }
}
