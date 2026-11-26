use core::f32::consts::SQRT_2;

use ::tmc2209::data::MicroStepResolution;
use ::tmc2209::reg;
use embedded_hal_async::delay::DelayNs;
use embedded_io_async::{Read, Write};

use super::bus::{Bus, Error};

/// Sense resistors fitted on the Omnimotion board.
pub const R_SENSE: f32 = 0.11;

/// What a healthy TMC2209 reports in `IOIN.version`.
pub const VERSION: u8 = 0x21;

/// Continuous current limit for the board's cooling.
pub const MAX_CURRENT_MA: f32 = 1_400.0;

pub const MAX_THRESHOLD: u32 = 0x000F_FFFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    InvalidCurrent,
    InvalidMicrosteps,
    InvalidRate,
    ThresholdOutOfRange,
}

/// Bit times the driver waits before replying, so the shared wire can turn around.
const SEND_DELAY: u8 = 2;

/// Standstill current ramp, in multiples of 2^18 clocks. At least 2 is required
/// for StealthChop to auto-tune its offset.
const IHOLD_DELAY: u8 = 4;

/// Delay from standstill to current reduction, in multiples of 2^18 clocks.
const TPOWERDOWN: u8 = 20;

#[derive(Clone, Copy, Debug)]
pub struct DriverConfig {
    pub run_current_ma: f32,
    pub hold_current_ma: f32,
    pub microsteps: MicroStepResolution,
    pub interpolate: bool,
    pub stealthchop: bool,
    /// TSTEP below which SpreadCycle takes over; zero stays in StealthChop.
    pub stealthchop_threshold: u32,
    pub reverse: bool,
}

impl Default for DriverConfig {
    fn default() -> Self {
        Self {
            run_current_ma: 800.0,
            hold_current_ma: 400.0,
            microsteps: MicroStepResolution::from_microsteps(16),
            interpolate: true,
            stealthchop: true,
            stealthchop_threshold: 0,
            reverse: false,
        }
    }
}

pub struct Driver<'a, S, D> {
    pub(crate) bus: &'a mut Bus<S, D>,
    pub(crate) address: u8,
}

impl<'a, S: Read + Write, D: DelayNs> Driver<'a, S, D> {
    pub fn new(bus: &'a mut Bus<S, D>, address: u8) -> Self {
        Self { bus, address }
    }

    pub fn address(&self) -> u8 {
        self.address
    }

    pub async fn version(&mut self) -> Result<u8, Error<S::Error>> {
        Ok(self.inputs().await?.version() as u8)
    }

    /// The levels the driver sees on its own pins, plus its version.
    pub async fn inputs(&mut self) -> Result<reg::IOIN, Error<S::Error>> {
        self.bus.read(self.address).await
    }

    pub async fn global_status(&mut self) -> Result<reg::GSTAT, Error<S::Error>> {
        self.bus.read(self.address).await
    }

    /// Whether the driver reset (power-on or supply dip) since the last call.
    /// A reset also restarts the microstep sequencer, so the rotor may have
    /// jumped up to two full steps when the outputs next came on.
    pub async fn take_reset(&mut self) -> Result<bool, Error<S::Error>> {
        let status = self.global_status().await?;
        if status.reset() {
            self.bus.write(self.address, reg::GSTAT(1)).await?;
        }
        Ok(status.reset())
    }

    /// Reads GSTAT to acknowledge reset, clears `drv_err`, and rejects live faults.
    pub async fn clear_global_status(&mut self) -> Result<(), Error<S::Error>> {
        let status = self.global_status().await?;
        if status.uv_cp() {
            return Err(Error::DriverFault);
        }
        if status.drv_err() {
            let mut clear = reg::GSTAT::default();
            clear.set_drv_err(true);
            self.bus.write(self.address, clear).await?;
            let status = self.global_status().await?;
            if status.drv_err() || status.uv_cp() {
                return Err(Error::DriverFault);
            }
        }
        Ok(())
    }

    pub async fn apply(&mut self, config: &DriverConfig) -> Result<(), Error<S::Error>> {
        validate_currents(config.run_current_ma, config.hold_current_ma)?;
        validate_threshold(config.stealthchop_threshold)?;
        self.clear_global_status().await?;

        let mut slaveconf = reg::SLAVECONF(0);
        slaveconf.set_send_delay(SEND_DELAY);
        self.bus.write(self.address, slaveconf).await?;

        let (vsense, run, hold) = current_scales(config.run_current_ma, config.hold_current_ma);
        self.write_currents(run, hold).await?;

        let mut chopconf = reg::CHOPCONF::default();
        chopconf.set_mres(config.microsteps);
        chopconf.set_intpol(config.interpolate);
        chopconf.set_vsense(vsense);
        self.write_chopconf(chopconf).await?;

        let mut gconf = reg::GCONF(0);
        // MS1/MS2 are wired as address straps, so microstepping has to come from
        // CHOPCONF.MRES rather than the pins.
        gconf.set_mstep_reg_select(true);
        // i_scale_analog resets to 1, which would leave the 33k/10k VREF divider
        // in charge of the current instead of IHOLD_IRUN.
        gconf.set_i_scale_analog(false);
        gconf.set_pdn_disable(true);
        gconf.set_en_spread_cycle(!config.stealthchop);
        gconf.set_shaft(config.reverse);
        gconf.set_multistep_filt(true);
        self.bus.write(self.address, gconf).await?;

        let mut powerdown = reg::TPOWERDOWN(0);
        powerdown.set(TPOWERDOWN);
        self.bus.write(self.address, powerdown).await?;

        self.set_stealthchop_threshold(config.stealthchop_threshold)
            .await
    }

    pub async fn set_current(&mut self, run_ma: f32, hold_ma: f32) -> Result<(), Error<S::Error>> {
        validate_currents(run_ma, hold_ma)?;
        let (vsense, run, hold) = current_scales(run_ma, hold_ma);

        let mut chopconf = self.chopconf().await?;
        if vsense && !chopconf.vsense() {
            chopconf.set_vsense(true);
            self.write_chopconf(chopconf).await?;
            self.write_currents(run, hold).await
        } else {
            self.write_currents(run, hold).await?;
            if chopconf.vsense() != vsense {
                chopconf.set_vsense(vsense);
                self.write_chopconf(chopconf).await?;
            }
            Ok(())
        }
    }

    pub async fn set_microsteps(
        &mut self,
        microsteps: MicroStepResolution,
    ) -> Result<(), Error<S::Error>> {
        let mut chopconf = self.chopconf().await?;
        chopconf.set_mres(microsteps);
        self.write_chopconf(chopconf).await
    }

    pub async fn set_stealthchop_threshold(&mut self, tstep: u32) -> Result<(), Error<S::Error>> {
        validate_threshold(tstep)?;
        let mut threshold = reg::TPWMTHRS(0);
        threshold.set(tstep);
        self.bus.write(self.address, threshold).await
    }

    pub async fn set_spreadcycle(&mut self, enabled: bool) -> Result<(), Error<S::Error>> {
        let mut gconf: reg::GCONF = self.bus.read(self.address).await?;
        gconf.set_en_spread_cycle(enabled);
        self.bus.write(self.address, gconf).await
    }

    /// Falls back to the wire only until `apply` has established the shadow.
    async fn chopconf(&mut self) -> Result<reg::CHOPCONF, Error<S::Error>> {
        match self.bus.cached_chopconf(self.address) {
            Some(chopconf) => Ok(chopconf),
            None => self.bus.read(self.address).await,
        }
    }

    async fn write_chopconf(&mut self, chopconf: reg::CHOPCONF) -> Result<(), Error<S::Error>> {
        self.bus.write(self.address, chopconf).await?;
        self.bus.cache_chopconf(self.address, chopconf);
        Ok(())
    }

    async fn write_currents(&mut self, run: u8, hold: u8) -> Result<(), Error<S::Error>> {
        let mut current = reg::IHOLD_IRUN(0);
        current.set_irun(run);
        current.set_ihold(hold);
        current.set_ihold_delay(IHOLD_DELAY);
        self.bus.write(self.address, current).await
    }
}

pub fn microstep_resolution(microsteps: u32) -> Result<MicroStepResolution, ConfigError> {
    if microsteps.is_power_of_two() && microsteps <= 256 {
        Ok(MicroStepResolution::from_microsteps(microsteps))
    } else {
        Err(ConfigError::InvalidMicrosteps)
    }
}

pub(crate) fn validate_threshold(value: u32) -> Result<(), ConfigError> {
    if value <= MAX_THRESHOLD {
        Ok(())
    } else {
        Err(ConfigError::ThresholdOutOfRange)
    }
}

fn validate_currents(run_ma: f32, hold_ma: f32) -> Result<(), ConfigError> {
    if run_ma.is_finite()
        && hold_ma.is_finite()
        && run_ma > 0.0
        && hold_ma >= 0.0
        && hold_ma <= run_ma
        && run_ma <= MAX_CURRENT_MA
    {
        Ok(())
    } else {
        Err(ConfigError::InvalidCurrent)
    }
}

/// Run and hold share a single vsense bit, so the hold current has to be scaled
/// against whatever full-scale voltage the run current selected.
pub fn current_scales(run_ma: f32, hold_ma: f32) -> (bool, u8, u8) {
    let (vsense, run) = ::tmc2209::rms_current_to_vsense_cs(R_SENSE, run_ma);
    (vsense, run, current_scale(hold_ma, vsense))
}

pub fn current_scale(milliamps: f32, vsense: bool) -> u8 {
    let full_scale = if vsense { 0.180 } else { 0.325 };
    let scale = 32.0 * SQRT_2 * (milliamps / 1_000.0) * (R_SENSE + 0.02) / full_scale - 1.0;

    if scale < 0.0 {
        0
    } else if scale > 31.0 {
        31
    } else {
        scale as u8
    }
}
