//! The slice of esp-hal 1.1 that the Omnimotion board code and the applications on it use, backed by the
//! simulated board in `omnimotion-sim`. Signatures follow esp-hal so the production sources compile
//! unchanged.

#![allow(clippy::new_without_default)]

pub use esp_hal_macros::{handler, main, ram};

use core::marker::PhantomData;

pub mod analog;
pub mod gpio;
pub mod i2c;
pub mod regs;
pub mod timer;
pub mod uart;

/// Driver mode markers.
pub struct Blocking;
pub struct Async;

#[derive(Default)]
pub struct Config {
    _private: (),
}

macro_rules! peripherals {
    ($($name:ident),* $(,)?) => {
        pub mod peripherals {
            use core::marker::PhantomData;
            $(
                #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
                pub struct $name<'d>(pub(crate) PhantomData<&'d mut ()>);
                impl $name<'_> {
                    /// # Safety
                    /// As in esp-hal: the caller makes sure the peripheral is not used twice.
                    pub unsafe fn steal() -> Self {
                        Self(PhantomData)
                    }
                }
            )*
        }

        #[allow(non_snake_case)]
        pub struct Peripherals {
            $(pub $name: peripherals::$name<'static>,)*
        }

        impl Peripherals {
            fn take() -> Self {
                Self { $($name: peripherals::$name(PhantomData),)* }
            }
        }
    };
}

macro_rules! pins {
    ($($name:ident = $n:expr),* $(,)?) => {
        $(
            impl<'d> From<peripherals::$name<'d>> for gpio::AnyPin<'d> {
                fn from(_: peripherals::$name<'d>) -> Self {
                    gpio::AnyPin::new($n)
                }
            }
        )*
    };
}

// Only the pins the board code and the firmware take.
peripherals!(
    GPIO1,
    GPIO4,
    GPIO5,
    GPIO6,
    GPIO7,
    GPIO8,
    GPIO9,
    GPIO10,
    GPIO11,
    GPIO12,
    GPIO13,
    GPIO14,
    GPIO15,
    GPIO16,
    GPIO17,
    GPIO18,
    GPIO21,
    GPIO38,
    GPIO39,
    GPIO40,
    GPIO41,
    GPIO47,
    GPIO,
    I2C0,
    UART1,
    ADC1,
    TIMG0,
    FLASH,
    WIFI,
    SYSTIMER,
    SW_INTERRUPT,
    CPU_CTRL,
);

pins!(
    GPIO4 = 4,
    GPIO5 = 5,
    GPIO6 = 6,
    GPIO7 = 7,
    GPIO8 = 8,
    GPIO9 = 9,
    GPIO10 = 10,
    GPIO11 = 11,
    GPIO12 = 12,
    GPIO13 = 13,
    GPIO14 = 14,
    GPIO15 = 15,
    GPIO16 = 16,
    GPIO17 = 17,
    GPIO18 = 18,
    GPIO21 = 21,
    GPIO38 = 38,
    GPIO39 = 39,
    GPIO40 = 40,
    GPIO41 = 41,
    GPIO47 = 47,
);

/// The chip comes out of reset: the world must already be installed by the host.
pub fn init(_config: Config) -> Peripherals {
    omnimotion_sim::hal::console("ESP-ROM: esp32s3 (simulated) boot");
    Peripherals::take()
}

pub mod system {
    pub fn software_reset() -> ! {
        omnimotion_sim::reset("software reset")
    }

    /// The simulated chip keeps no reset cause.
    #[derive(Clone, Copy, Debug)]
    pub enum SocResetReason {}

    pub fn reset_reason() -> Option<SocResetReason> {
        None
    }
}

pub mod time {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Rate(u32);

    impl Rate {
        pub const fn from_hz(hz: u32) -> Self {
            Self(hz)
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Duration(u64);

    impl Duration {
        pub const fn from_micros(us: u64) -> Self {
            Self(us)
        }
        pub const fn from_secs(s: u64) -> Self {
            Self(s * 1_000_000)
        }
        pub const fn as_micros(&self) -> u64 {
            self.0
        }
    }
}

pub mod interrupt {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Priority {
        Priority3,
    }
}

impl peripherals::GPIO<'_> {
    pub fn regs() -> &'static regs::GpioRegs {
        &regs::GpioRegs
    }
}

impl peripherals::TIMG0<'_> {
    pub fn regs() -> &'static regs::TimgRegs {
        &regs::TimgRegs
    }
}
