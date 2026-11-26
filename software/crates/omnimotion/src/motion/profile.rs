use libm::sqrtf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitsError {
    NonFinite,
    NonPositive,
    MinimumExceedsMaximum,
}

/// Master step rate limits, in steps per second and steps per second squared.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub max_rate: f32,
    /// The rate a move starts and stops at; a stepper can jump to this unramped.
    pub min_rate: f32,
    pub accel: f32,
}

impl Limits {
    pub fn validate(&self) -> Result<(), LimitsError> {
        if !self.max_rate.is_finite() || !self.min_rate.is_finite() || !self.accel.is_finite() {
            return Err(LimitsError::NonFinite);
        }
        if self.max_rate <= 0.0 || self.min_rate <= 0.0 || self.accel <= 0.0 {
            return Err(LimitsError::NonPositive);
        }
        if self.min_rate > self.max_rate {
            return Err(LimitsError::MinimumExceedsMaximum);
        }
        Ok(())
    }
}

pub trait Profile {
    fn rate(&self, travelled: u64, total: u64, limits: &Limits) -> f32;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Trapezoid;

impl Profile for Trapezoid {
    fn rate(&self, travelled: u64, total: u64, limits: &Limits) -> f32 {
        let remaining = total.saturating_sub(travelled);
        let start = limits.min_rate * limits.min_rate;

        // v² = v0² + 2as, forwards from the start and backwards from the end. The
        // lower of the two is the cruise plateau, or a triangle if they cross
        // below max_rate.
        let accelerating = sqrtf(start + 2.0 * limits.accel * travelled as f32);
        let decelerating = sqrtf(start + 2.0 * limits.accel * remaining as f32);

        accelerating
            .min(decelerating)
            .min(limits.max_rate)
            .max(limits.min_rate)
    }
}
