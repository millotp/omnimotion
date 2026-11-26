#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotaryError {
    ZeroPeriod,
    ZeroDivisions,
    Overflow,
}

/// A rotary period that can always be represented by signed motion counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RotaryPeriod(i64);

impl RotaryPeriod {
    pub fn new(steps: u64) -> Result<Self, RotaryError> {
        if steps == 0 {
            return Err(RotaryError::ZeroPeriod);
        }
        let steps = i64::try_from(steps).map_err(|_| RotaryError::Overflow)?;
        Ok(Self(steps))
    }

    pub const fn steps(self) -> i64 {
        self.0
    }

    /// Maps an unbounded signed counter into `0..period`.
    pub fn normalize(self, raw: i64) -> i64 {
        raw.rem_euclid(self.0)
    }

    /// Returns the shortest signed delta to an absolute modulo target.
    ///
    /// An exact half turn is resolved in the positive direction.
    pub fn shortest_delta(self, current: i64, target: i64) -> i64 {
        let current = self.normalize(current);
        let target = self.normalize(target);
        let forward = (target - current).rem_euclid(self.0);
        if forward <= self.0 / 2 {
            forward
        } else {
            forward - self.0
        }
    }

    /// Computes an absolute target as a rational fraction of one revolution.
    ///
    /// The result is rounded toward zero from the exact non-negative fraction.
    /// Computing every target independently prevents cumulative rounding drift.
    pub fn fraction_target(self, index: u64, divisions: u64) -> Result<i64, RotaryError> {
        if divisions == 0 {
            return Err(RotaryError::ZeroDivisions);
        }
        let index = index % divisions;
        let target = u128::from(index) * self.0 as u128 / u128::from(divisions);
        i64::try_from(target).map_err(|_| RotaryError::Overflow)
    }
}
