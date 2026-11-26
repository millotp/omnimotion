use crate::motion::profile::{Limits, LimitsError};
use crate::AXES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitError {
    NonFinite,
    NonPositive,
    MinimumExceedsMaximum,
    CoordinateOverflow,
    EmptyMove,
    InvalidLimits(LimitsError),
}

/// Per-axis calibration and limits, in whatever unit the axis uses: millimetres
/// for a linear axis, degrees for a rotary one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisConfig {
    pub steps_per_unit: f32,
    /// Units per second.
    pub max_velocity: f32,
    /// Units per second squared.
    pub max_acceleration: f32,
    /// Units per second the axis may start and stop at without a ramp.
    pub min_velocity: f32,
}

impl AxisConfig {
    pub fn validate(&self) -> Result<(), UnitError> {
        if !self.steps_per_unit.is_finite()
            || !self.max_velocity.is_finite()
            || !self.max_acceleration.is_finite()
            || !self.min_velocity.is_finite()
        {
            return Err(UnitError::NonFinite);
        }
        if self.steps_per_unit <= 0.0
            || self.max_velocity <= 0.0
            || self.max_acceleration <= 0.0
            || self.min_velocity <= 0.0
        {
            return Err(UnitError::NonPositive);
        }
        if self.min_velocity > self.max_velocity {
            return Err(UnitError::MinimumExceedsMaximum);
        }
        Ok(())
    }

    pub fn steps(&self, units: f32) -> Result<i64, UnitError> {
        self.validate()?;
        if !units.is_finite() {
            return Err(UnitError::NonFinite);
        }
        let steps = libm::round(units as f64 * self.steps_per_unit as f64);
        const I64_MAX_EXCLUSIVE: f64 = 9_223_372_036_854_775_808.0;
        if !steps.is_finite() || !(-I64_MAX_EXCLUSIVE..I64_MAX_EXCLUSIVE).contains(&steps) {
            return Err(UnitError::CoordinateOverflow);
        }
        Ok(steps as i64)
    }

    pub fn units(&self, steps: i64) -> Result<f32, UnitError> {
        self.validate()?;
        let units = steps as f64 / self.steps_per_unit as f64;
        if units < -f32::MAX as f64 || units > f32::MAX as f64 {
            return Err(UnitError::CoordinateOverflow);
        }
        Ok(units as f32)
    }

    pub fn limits(&self) -> Result<Limits, UnitError> {
        self.validate()?;
        let limits = Limits {
            max_rate: self.max_velocity * self.steps_per_unit,
            min_rate: self.min_velocity * self.steps_per_unit,
            accel: self.max_acceleration * self.steps_per_unit,
        };
        limits.validate().map_err(UnitError::InvalidLimits)?;
        Ok(limits)
    }
}

pub fn steps_per_mm(
    full_steps_per_rev: u32,
    microsteps: u32,
    mm_per_rev: f32,
) -> Result<f32, UnitError> {
    if full_steps_per_rev == 0 || microsteps == 0 || !mm_per_rev.is_finite() || mm_per_rev <= 0.0 {
        return Err(if mm_per_rev.is_finite() {
            UnitError::NonPositive
        } else {
            UnitError::NonFinite
        });
    }
    let product = full_steps_per_rev
        .checked_mul(microsteps)
        .ok_or(UnitError::CoordinateOverflow)?;
    let result = product as f32 / mm_per_rev;
    if !result.is_finite() {
        return Err(UnitError::CoordinateOverflow);
    }
    Ok(result)
}

/// Limits for the master step rate of a coordinated move. Each axis covers
/// `delta[i]` steps while the master covers `max(delta)`, so an axis constrains
/// the master rate by `limit_i * max(delta) / delta[i]`; the tightest wins.
pub fn group_limits(
    configs: &[AxisConfig; AXES],
    delta: &[i64; AXES],
) -> Result<Limits, UnitError> {
    let master = delta.iter().map(|d| d.unsigned_abs()).max().unwrap_or(0);
    if master == 0 {
        return Err(UnitError::EmptyMove);
    }

    let mut limits = Limits {
        max_rate: f32::INFINITY,
        min_rate: f32::INFINITY,
        accel: f32::INFINITY,
    };

    for (config, steps) in configs.iter().zip(delta) {
        let steps = steps.unsigned_abs();
        if steps == 0 {
            continue;
        }
        let scale = master as f32 / steps as f32;
        let axis = config.limits()?;
        limits.max_rate = limits.max_rate.min(axis.max_rate * scale);
        limits.min_rate = limits.min_rate.min(axis.min_rate * scale);
        limits.accel = limits.accel.min(axis.accel * scale);
    }

    limits.validate().map_err(UnitError::InvalidLimits)?;
    Ok(limits)
}
