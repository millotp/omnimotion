use core::marker::PhantomData;

pub use omnimotion_sim::hw::Pull;

pub struct AnyPin<'d> {
    number: u8,
    _p: PhantomData<&'d mut ()>,
}

impl AnyPin<'_> {
    pub(crate) fn new(number: u8) -> Self {
        Self {
            number,
            _p: PhantomData,
        }
    }
}

pub trait Pin {
    fn number(&self) -> u8;
}

impl Pin for AnyPin<'_> {
    fn number(&self) -> u8 {
        self.number
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Low,
    High,
}

#[derive(Clone, Copy, Default)]
pub struct InputConfig {
    pull: Pull,
}

impl InputConfig {
    pub fn with_pull(self, pull: Pull) -> Self {
        Self { pull }
    }
}

#[derive(Clone, Copy, Default)]
pub struct OutputConfig;

pub struct Input<'d> {
    pin: u8,
    _p: PhantomData<&'d mut ()>,
}

impl<'d> Input<'d> {
    pub fn new(pin: impl Into<AnyPin<'d>>, config: InputConfig) -> Self {
        let pin = pin.into().number;
        omnimotion_sim::hal::configure_input(pin, config.pull);
        Self {
            pin,
            _p: PhantomData,
        }
    }

    pub fn is_high(&self) -> bool {
        omnimotion_sim::hal::input(self.pin)
    }

    pub fn is_low(&self) -> bool {
        !self.is_high()
    }
}

pub struct Output<'d> {
    pin: u8,
    _p: PhantomData<&'d mut ()>,
}

impl<'d> Output<'d> {
    pub fn new(pin: impl Into<AnyPin<'d>>, level: Level, _config: OutputConfig) -> Self {
        let mut out = Self {
            pin: pin.into().number,
            _p: PhantomData,
        };
        out.set_level(level);
        out
    }

    pub fn set_level(&mut self, level: Level) {
        let bit = 1u64 << self.pin;
        match level {
            Level::High => omnimotion_sim::hal::gpio_update(bit, 0),
            Level::Low => omnimotion_sim::hal::gpio_update(0, bit),
        }
    }

    pub fn set_high(&mut self) {
        self.set_level(Level::High)
    }

    pub fn set_low(&mut self) {
        self.set_level(Level::Low)
    }

    pub fn is_set_high(&self) -> bool {
        omnimotion_sim::hal::gpio_out() & (1 << self.pin) != 0
    }
}
