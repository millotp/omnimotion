pub mod master {
    use core::marker::PhantomData;

    use embedded_hal_async::i2c::{ErrorKind, NoAcknowledgeSource, Operation, SevenBitAddress};

    use crate::gpio::AnyPin;
    use crate::peripherals::I2C0;
    use crate::time::Rate;

    #[derive(Clone, Copy, Default)]
    pub struct Config;

    impl Config {
        pub fn with_frequency(self, _rate: Rate) -> Self {
            self
        }
    }

    #[derive(Debug)]
    pub struct ConfigError;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Error {
        AcknowledgeCheckFailed,
    }

    impl embedded_hal_async::i2c::Error for Error {
        fn kind(&self) -> ErrorKind {
            ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address)
        }
    }

    /// Nothing is plugged into the OLED header, so every address goes unacknowledged.
    pub struct I2c<'d, Dm> {
        _p: PhantomData<(&'d mut (), Dm)>,
    }

    impl<'d> I2c<'d, crate::Blocking> {
        pub fn new(_i2c: I2C0<'d>, _config: Config) -> Result<Self, ConfigError> {
            Ok(Self { _p: PhantomData })
        }

        pub fn into_async(self) -> I2c<'d, crate::Async> {
            I2c { _p: PhantomData }
        }
    }

    impl<'d, Dm> I2c<'d, Dm> {
        pub fn with_sda(self, _pin: impl Into<AnyPin<'d>>) -> Self {
            self
        }
        pub fn with_scl(self, _pin: impl Into<AnyPin<'d>>) -> Self {
            self
        }
    }

    impl<Dm> embedded_hal_async::i2c::ErrorType for I2c<'_, Dm> {
        type Error = Error;
    }

    impl embedded_hal_async::i2c::I2c<SevenBitAddress> for I2c<'_, crate::Async> {
        async fn transaction(
            &mut self,
            _address: u8,
            _operations: &mut [Operation<'_>],
        ) -> Result<(), Error> {
            Err(Error::AcknowledgeCheckFailed)
        }
    }
}
