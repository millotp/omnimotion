use crate::motion::profile::{Limits, LimitsError, Profile};
use crate::{AxisMask, AXES};

/// Short enough that the rate tracks the ramp, long enough that a 16-deep queue
/// covers plenty of lookahead.
const SLICE_SECONDS: f32 = 0.001;

/// The Bresenham parameters of a move, carried by its first segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoveStart {
    pub delta: [u64; AXES],
    pub dir: AxisMask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    /// NCO increment per engine tick.
    pub rate: u32,
    /// Master steps this segment emits.
    pub steps: u32,
    pub start: Option<MoveStart>,
    pub last: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanError {
    EmptyMove,
    InvalidLimits(LimitsError),
    UnsupportedTickRate,
    RateExceedsTickRate,
}

/// Slices a move into segments of constant rate. The step counts always sum to
/// the master step total, so position is exact however the rates round.
pub struct Planner<P> {
    profile: P,
    limits: Limits,
    tick_hz: u32,
    total: u64,
    travelled: u64,
    start: Option<MoveStart>,
}

impl<P: Profile> Planner<P> {
    pub fn new(
        delta: [i64; AXES],
        limits: Limits,
        tick_hz: u32,
        profile: P,
    ) -> Result<Self, PlanError> {
        limits.validate().map_err(PlanError::InvalidLimits)?;
        if tick_hz == 0 {
            return Err(PlanError::UnsupportedTickRate);
        }
        if limits.max_rate > tick_hz as f32 {
            return Err(PlanError::RateExceedsTickRate);
        }

        let mut steps = [0u64; AXES];
        let mut dir = 0;
        for axis in 0..AXES {
            steps[axis] = delta[axis].unsigned_abs();
            if delta[axis] >= 0 {
                dir |= 1 << axis;
            }
        }

        let total = steps.iter().copied().max().unwrap_or(0);
        if total == 0 {
            return Err(PlanError::EmptyMove);
        }

        Ok(Self {
            profile,
            limits,
            tick_hz,
            total,
            travelled: 0,
            start: Some(MoveStart { delta: steps, dir }),
        })
    }

    pub fn total_steps(&self) -> u64 {
        self.total
    }

    /// Ends the move as soon as the ramp allows: what is left becomes the
    /// shortest stop from the current rate, so the profile decelerates to
    /// `min_rate` from here. The final segment then ends the move short of its
    /// target.
    pub fn stop(&mut self) {
        let rate = self.profile.rate(self.travelled, self.total, &self.limits);
        let floor = self.limits.min_rate;
        let stopping = (rate * rate - floor * floor).max(0.0) / (2.0 * self.limits.accel);
        let stopping = (libm::ceilf(stopping) as u64).max(1);
        self.total = self.total.min(self.travelled + stopping);
    }
}

impl<P: Profile> Iterator for Planner<P> {
    type Item = Result<Segment, PlanError>;

    fn next(&mut self) -> Option<Self::Item> {
        let remaining = self.total - self.travelled;
        if remaining == 0 {
            return None;
        }

        let start_rate = self.profile.rate(self.travelled, self.total, &self.limits);
        let steps = ((start_rate * SLICE_SECONDS) as u64)
            .clamp(1, remaining)
            .min(u32::MAX as u64);
        let end_rate = self
            .profile
            .rate(self.travelled + steps, self.total, &self.limits);
        // The lower endpoint prevents a constant-rate slice overshooting the ramp.
        let rate = match (
            nco_increment(start_rate, self.tick_hz),
            nco_increment(end_rate, self.tick_hz),
        ) {
            (Ok(start), Ok(end)) => start.min(end),
            (Err(error), _) | (_, Err(error)) => {
                self.travelled = self.total;
                return Some(Err(error));
            }
        };
        self.travelled += steps;

        Some(Ok(Segment {
            rate,
            steps: steps as u32,
            start: self.start.take(),
            last: self.travelled == self.total,
        }))
    }
}

pub fn nco_increment(rate_hz: f32, tick_hz: u32) -> Result<u32, PlanError> {
    if tick_hz == 0 {
        return Err(PlanError::UnsupportedTickRate);
    }
    if !rate_hz.is_finite() || rate_hz <= 0.0 {
        return Err(PlanError::InvalidLimits(if rate_hz.is_finite() {
            LimitsError::NonPositive
        } else {
            LimitsError::NonFinite
        }));
    }
    if rate_hz > tick_hz as f32 {
        return Err(PlanError::RateExceedsTickRate);
    }

    let increment = rate_hz as f64 / tick_hz as f64 * 4_294_967_296.0;
    Ok(increment.clamp(1.0, u32::MAX as f64) as u32)
}
