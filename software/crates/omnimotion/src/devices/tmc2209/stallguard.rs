use ::tmc2209::reg;
use embedded_hal_async::delay::DelayNs;
use embedded_io_async::{Read, Write};

use super::bus::Error;
use super::config::{validate_threshold, ConfigError, Driver, MAX_THRESHOLD};

/// TSTEP counts driver clocks between microsteps and is always expressed at
/// 1/256 resolution, whatever `CHOPCONF.MRES` is set to.
pub fn tstep_for_rate(rate_hz: f32, microsteps: u32) -> Result<u32, ConfigError> {
    if !rate_hz.is_finite() || rate_hz <= 0.0 || !microsteps.is_power_of_two() || microsteps > 256 {
        return Err(if rate_hz.is_finite() && rate_hz > 0.0 {
            ConfigError::InvalidMicrosteps
        } else {
            ConfigError::InvalidRate
        });
    }

    let at_full_resolution = rate_hz * (256 / microsteps) as f32;
    let tstep = ::tmc2209::INTERNAL_CLOCK_HZ / at_full_resolution;
    if !tstep.is_finite() || tstep < 1.0 || tstep > MAX_THRESHOLD as f32 {
        Err(ConfigError::ThresholdOutOfRange)
    } else {
        Ok(tstep as u32)
    }
}

impl<S: Read + Write, D: DelayNs> Driver<'_, S, D> {
    /// 255 is the most sensitive setting and 0 the least.
    pub async fn set_stallguard_threshold(&mut self, threshold: u8) -> Result<(), Error<S::Error>> {
        let mut sgthrs = reg::SGTHRS(0);
        sgthrs.set(threshold);
        self.bus.write(self.address, sgthrs).await
    }

    /// StallGuard and CoolStep only run above this velocity; below it DIAG stays
    /// quiet however hard the motor stalls.
    pub async fn set_coolstep_threshold(&mut self, tstep: u32) -> Result<(), Error<S::Error>> {
        validate_threshold(tstep)?;
        let mut tcoolthrs = reg::TCOOLTHRS(0);
        tcoolthrs.set(tstep);
        self.bus.write(self.address, tcoolthrs).await
    }

    pub async fn stallguard_result(&mut self) -> Result<u16, Error<S::Error>> {
        let result: reg::SG_RESULT = self.bus.read(self.address).await?;
        Ok(result.get())
    }

    pub async fn tstep(&mut self) -> Result<u32, Error<S::Error>> {
        let tstep: reg::TSTEP = self.bus.read(self.address).await?;
        Ok(tstep.get())
    }

    pub async fn status(&mut self) -> Result<reg::DRV_STATUS, Error<S::Error>> {
        self.bus.read(self.address).await
    }
}
