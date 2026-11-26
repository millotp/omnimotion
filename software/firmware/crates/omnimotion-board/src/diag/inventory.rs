//! Startup survey of a board that is only ever partly populated.

use core::fmt;

use embedded_hal_async::i2c::I2c;
use esp_hal::analog::adc::{Adc, AdcCalCurve, AdcConfig, AdcPin, Attenuation};
use esp_hal::peripherals::{ADC1, GPIO1};
use omnimotion::devices::power::{PowerSource, Rails, Volts};
use omnimotion::devices::tmc2209::{Driver, ADDRESSES, VERSION};
use omnimotion::AXES;

use crate::pins;
use crate::TmcBus;

/// What one of the driver sockets had to say over the shared UART.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverPresence {
    /// Replied with the 0x21 a TMC2209 reports in `IOIN.version`.
    Responding,
    Mismatched(u8),
    /// No reply: either the driver is not fitted or VMOTOR is down, which the
    /// UART alone cannot tell apart.
    Missing,
}

/// Everything the board could work out about itself at startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inventory {
    /// The address a display answered on, since SSD1306 modules strap either.
    pub oled: Option<u8>,
    pub rails: Rails,
    pub estop_closed: bool,
    pub drivers: [DriverPresence; AXES],
}

impl Inventory {
    pub fn source(&self) -> PowerSource {
        self.rails.source()
    }

    pub fn drivers_responding(&self) -> usize {
        self.drivers
            .iter()
            .filter(|driver| **driver == DriverPresence::Responding)
            .count()
    }

    pub fn responding_axes(&self) -> u8 {
        self.drivers
            .iter()
            .enumerate()
            .fold(0, |mask, (axis, driver)| {
                mask | u8::from(*driver == DriverPresence::Responding) << axis
            })
    }

    pub fn can_run_motors(&self) -> bool {
        self.rails.source() == PowerSource::Bulk && self.estop_closed
    }
}

type VmonPin = AdcPin<GPIO1<'static>, ADC1<'static>, AdcCalCurve<ADC1<'static>>>;

/// The VMOTOR monitor: a 100 k / 10 k divider into ADC1_CH0.
pub struct Vmon {
    adc: Adc<'static, ADC1<'static>, esp_hal::Blocking>,
    pin: VmonPin,
}

impl Vmon {
    pub(crate) fn new(adc: ADC1<'static>, pin: GPIO1<'static>) -> Self {
        let mut config = AdcConfig::new();
        // The curve scheme reads the chip's eFuse calibration and hands back
        // millivolts at the pin rather than raw counts.
        let pin =
            config.enable_pin_with_cal::<_, AdcCalCurve<ADC1<'static>>>(pin, Attenuation::_11dB);
        Self {
            adc: Adc::new(adc, config),
            pin,
        }
    }

    pub fn read(&mut self) -> Rails {
        Rails::from_pin_mv(i32::from(self.adc.read_blocking(&mut self.pin)))
    }
}

/// An address-only write is all the evidence an SSD1306 can give: it has no
/// register worth reading.
pub(crate) async fn find_oled<I: I2c>(i2c: &mut I) -> Option<u8> {
    for address in pins::OLED_ADDRESSES {
        if i2c.write(address, &[]).await.is_ok() {
            return Some(address);
        }
    }
    None
}

pub(crate) async fn survey_drivers(tmc: &mut TmcBus) -> [DriverPresence; AXES] {
    let mut drivers = [DriverPresence::Missing; AXES];
    for (driver, address) in drivers.iter_mut().zip(ADDRESSES) {
        *driver = match Driver::new(tmc, address).version().await {
            Ok(VERSION) => DriverPresence::Responding,
            Ok(version) => DriverPresence::Mismatched(version),
            Err(_) => DriverPresence::Missing,
        };
    }
    drivers
}

impl fmt::Display for DriverPresence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Responding => f.write_str("tmc2209"),
            Self::Mismatched(version) => write!(f, "version {version:#04x}"),
            Self::Missing => f.write_str("silent"),
        }
    }
}

impl fmt::Display for Inventory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "board inventory")?;

        write!(f, "  power      ")?;
        match self.rails.source() {
            PowerSource::UsbOnly => writeln!(f, "USB only, no VMOTOR")?,
            PowerSource::Bulk => writeln!(f, "bulk {}", Volts(self.rails.motor_mv))?,
            PowerSource::BulkOutOfRange => writeln!(
                f,
                "bulk {} outside motor-ready window",
                Volts(self.rails.motor_mv)
            )?,
        }

        let estop = match self.estop_closed {
            true => "closed",
            false => "open",
        };
        writeln!(f, "  e-stop     {estop}")?;

        match self.oled {
            Some(address) => writeln!(f, "  oled       {address:#04x} responding")?,
            None => writeln!(f, "  oled       not fitted")?,
        }

        write!(f, "  drivers    ")?;
        for (axis, driver) in self.drivers.iter().enumerate() {
            if axis > 0 {
                write!(f, ", ")?;
            }
            write!(f, "M{axis} {driver}")?;
        }
        writeln!(f)
    }
}
