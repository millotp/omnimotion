pub mod homing;
pub mod stepper;
pub mod supervisor;

use embassy_time::{with_timeout, Duration};
use omnimotion::{
    group_limits, AxisConfig, Lifecycle, Limits, PlanError, Planner, RotaryError, RotaryPeriod,
    Trapezoid, UnitError, WriteGate, AXES, TICK_HZ,
};

use crate::storage::flash;
use stepper::Stepper;
use supervisor::Fault;

pub use omnimotion::{AbortState, AbortTicket};

/// Stays below the queue's roughly 15 ms motion reserve.
const REFILL_BACKSTOP: Duration = Duration::from_millis(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    /// Stopped early by abort, endstop or stall; the position counters stay exact.
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbortAcknowledgement {
    pub ticket: AbortTicket,
    /// Raw physical counters, unaffected by `Motion::zero`.
    pub position: [i64; AXES],
}

/// The raw step counters, readable from any core while a move runs. They are
/// unaffected by `Motion::zero`; subtract `Motion::origin` for positions.
pub fn raw_position() -> [i64; AXES] {
    stepper::position()
}

/// Cross-core request path into the step ISR. It does not borrow `Motion`.
#[derive(Clone, Copy, Debug)]
pub struct AbortHandle;

impl AbortHandle {
    pub fn request(&self) -> AbortTicket {
        stepper::abort()
    }

    /// Ends the running move early along its own deceleration ramp, so a
    /// heavy load stops without losing steps. Segments already queued (about
    /// 15 ms) still run first. Moves started afterwards are not affected.
    pub fn request_stop(&self) {
        stepper::request_stop()
    }

    pub fn state(&self) -> AbortState {
        stepper::abort_state()
    }

    pub async fn wait(&self, ticket: AbortTicket) -> AbortAcknowledgement {
        stepper::await_abort(ticket).await;
        AbortAcknowledgement {
            ticket,
            position: stepper::position(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionError {
    InvalidAxis,
    AxisUnavailable,
    Busy,
    FaultLatched,
    CoordinateOverflow,
    Units(UnitError),
    Planning(PlanError),
    Rotary(RotaryError),
}

impl From<UnitError> for MotionError {
    fn from(error: UnitError) -> Self {
        Self::Units(error)
    }
}

impl From<PlanError> for MotionError {
    fn from(error: PlanError) -> Self {
        Self::Planning(error)
    }
}

impl From<RotaryError> for MotionError {
    fn from(error: RotaryError) -> Self {
        Self::Rotary(error)
    }
}

pub struct Motion {
    stepper: Stepper,
    axes: [AxisConfig; AXES],
    available_axes: u8,
    origin: [i64; AXES],
}

impl Motion {
    pub(crate) fn new(stepper: Stepper, axes: [AxisConfig; AXES], available_axes: u8) -> Self {
        Self {
            stepper,
            axes,
            available_axes,
            origin: [0; AXES],
        }
    }

    pub fn axis_available(&self, axis: usize) -> bool {
        axis < AXES && self.available_axes & (1 << axis) != 0
    }

    pub fn available_axes(&self) -> u8 {
        self.available_axes
    }

    /// Physical counters maintained by the ISR, unaffected by logical zeroing.
    pub fn raw_position(&self) -> [i64; AXES] {
        stepper::position()
    }

    pub fn persistence_gate(&self, lifecycle: &Lifecycle) -> WriteGate {
        WriteGate {
            motion_drained: self.stepper.drained(),
            abort_pending: stepper::aborting(),
            lifecycle: lifecycle.state(),
            safe_boundary: lifecycle.at_safe_boundary(),
        }
    }

    pub fn abort_handle(&self) -> AbortHandle {
        AbortHandle
    }

    pub(crate) fn retain_available_axes(&mut self, axes: u8) {
        self.available_axes &= axes;
    }

    pub(crate) fn set_available_axes(&mut self, axes: u8) {
        self.available_axes = axes;
    }

    pub fn config(&self, axis: usize) -> Result<AxisConfig, MotionError> {
        self.axes.get(axis).copied().ok_or(MotionError::InvalidAxis)
    }

    pub fn position(&self) -> Result<[i64; AXES], MotionError> {
        let raw = stepper::position();
        let mut position = [0; AXES];
        for axis in 0..AXES {
            position[axis] = raw[axis]
                .checked_sub(self.origin[axis])
                .ok_or(MotionError::CoordinateOverflow)?;
        }
        Ok(position)
    }

    /// The raw counters that read as zero: `position()` is `raw_position()` minus this.
    pub fn origin(&self) -> [i64; AXES] {
        self.origin
    }

    /// Makes the current position of `axis` read as zero.
    pub fn zero(&mut self, axis: usize) -> Result<(), MotionError> {
        if axis >= AXES {
            return Err(MotionError::InvalidAxis);
        }
        if !self.axis_available(axis) {
            return Err(MotionError::AxisUnavailable);
        }
        if !self.stepper.drained() || stepper::aborting() {
            return Err(MotionError::Busy);
        }
        if stepper::underrun() {
            supervisor::latch(Fault::Underrun);
            return Err(MotionError::FaultLatched);
        }
        if stepper::engine_fault() {
            supervisor::latch(Fault::MotionEngine);
            return Err(MotionError::FaultLatched);
        }
        if supervisor::latched() != 0 {
            return Err(MotionError::FaultLatched);
        }
        self.origin[axis] = stepper::position()[axis];
        Ok(())
    }

    pub fn position_units(&self) -> Result<[f32; AXES], MotionError> {
        let steps = self.position()?;
        let mut units = [0.0; AXES];
        for axis in 0..AXES {
            units[axis] = self.axes[axis].units(steps[axis])?;
        }
        Ok(units)
    }

    pub async fn move_to(&mut self, target: [i64; AXES]) -> Result<Outcome, MotionError> {
        let here = self.position()?;
        let mut delta = [0; AXES];
        for axis in 0..AXES {
            delta[axis] = target[axis]
                .checked_sub(here[axis])
                .ok_or(MotionError::CoordinateOverflow)?;
        }
        self.move_by(delta).await
    }

    pub async fn move_to_units(&mut self, target: [f32; AXES]) -> Result<Outcome, MotionError> {
        let mut steps = [0; AXES];
        for axis in 0..AXES {
            steps[axis] = self.axes[axis].steps(target[axis])?;
        }
        self.move_to(steps).await
    }

    /// Moves one rotary axis to an absolute modulo target by the shortest path.
    pub async fn move_to_modulo(
        &mut self,
        axis: usize,
        target: i64,
        period_steps: u64,
    ) -> Result<Outcome, MotionError> {
        if axis >= AXES {
            return Err(MotionError::InvalidAxis);
        }
        if !self.axis_available(axis) {
            return Err(MotionError::AxisUnavailable);
        }
        let period = RotaryPeriod::new(period_steps)?;
        let delta = period.shortest_delta(self.position()?[axis], target);
        if delta == 0 {
            return Ok(Outcome::Completed);
        }
        let mut move_delta = [0; AXES];
        move_delta[axis] = delta;
        self.move_by(move_delta).await
    }

    pub async fn move_by(&mut self, delta: [i64; AXES]) -> Result<Outcome, MotionError> {
        let limits = group_limits(&self.axes, &delta)?;
        self.move_by_with(delta, limits).await
    }

    pub async fn jog(&mut self, axis: usize, units: f32) -> Result<Outcome, MotionError> {
        if axis >= AXES {
            return Err(MotionError::InvalidAxis);
        }
        let mut delta = [0; AXES];
        delta[axis] = self.axes[axis].steps(units)?;
        self.move_by(delta).await
    }

    /// Runs a move under explicit master-rate limits instead of the axis defaults.
    pub async fn move_by_with(
        &mut self,
        delta: [i64; AXES],
        limits: Limits,
    ) -> Result<Outcome, MotionError> {
        if delta
            .iter()
            .enumerate()
            .any(|(axis, steps)| *steps != 0 && !self.axis_available(axis))
        {
            return Err(MotionError::AxisUnavailable);
        }
        if stepper::underrun() {
            supervisor::latch(Fault::Underrun);
            return Err(MotionError::FaultLatched);
        }
        if stepper::engine_fault() {
            supervisor::latch(Fault::MotionEngine);
            return Err(MotionError::FaultLatched);
        }
        if supervisor::latched() != 0 {
            return Err(MotionError::FaultLatched);
        }
        let _moving = flash::Moving::begin().await.ok_or(MotionError::Busy)?;
        let abort_generation = stepper::abort_generation();
        if stepper::aborting() {
            stepper::await_abort(abort_generation).await;
            return Ok(Outcome::Aborted);
        }

        let start = self.position()?;
        let raw = stepper::position();
        let mut target = [0; AXES];
        for axis in 0..AXES {
            target[axis] = start[axis]
                .checked_add(delta[axis])
                .ok_or(MotionError::CoordinateOverflow)?;
            raw[axis]
                .checked_add(delta[axis])
                .ok_or(MotionError::CoordinateOverflow)?;
        }

        let stop_generation = stepper::stop_generation();
        let mut stopping = false;
        let mut planner = Planner::new(delta, limits, TICK_HZ, Trapezoid)?;
        let mut pending = planner.next().transpose()?;

        loop {
            if !stopping && stepper::stop_generation() != stop_generation {
                planner.stop();
                stopping = true;
            }
            let requested = stepper::abort_generation();
            if requested != abort_generation {
                stepper::await_abort(requested).await;
                return Ok(Outcome::Aborted);
            }
            if stepper::underrun() || stepper::engine_fault() {
                break;
            }

            while let Some(segment) = pending {
                match self.stepper.enqueue(segment, abort_generation) {
                    Ok(()) => {
                        if stepper::abort_generation() != abort_generation {
                            stepper::await_abort(stepper::abort_generation()).await;
                            return Ok(Outcome::Aborted);
                        }
                        pending = match planner.next().transpose() {
                            Ok(segment) => segment,
                            Err(error) => {
                                let drain = stepper::abort();
                                stepper::await_abort(drain).await;
                                return Err(error.into());
                            }
                        };
                    }
                    Err(rejected) => {
                        pending = Some(rejected);
                        break;
                    }
                }
            }

            if pending.is_some() {
                let _ = with_timeout(REFILL_BACKSTOP, self.stepper.room()).await;
            } else if self.stepper.drained() {
                break;
            } else {
                let _ = with_timeout(REFILL_BACKSTOP, self.stepper.finish()).await;
            }
        }

        let requested = stepper::abort_generation();
        if requested != abort_generation {
            stepper::await_abort(requested).await;
            return Ok(Outcome::Aborted);
        }

        if stepper::underrun() {
            supervisor::latch(Fault::Underrun);
            return Err(MotionError::FaultLatched);
        }
        if stepper::engine_fault() {
            supervisor::latch(Fault::MotionEngine);
            return Err(MotionError::FaultLatched);
        }
        if supervisor::latched() != 0 {
            return Err(MotionError::FaultLatched);
        }
        if self.position()? == target {
            Ok(Outcome::Completed)
        } else {
            Ok(Outcome::Aborted)
        }
    }

    pub(crate) fn hold_lines(&mut self, axis: usize) -> Option<stepper::HeldLines<'_>> {
        self.stepper.hold_lines(axis)
    }

    pub(crate) fn clear_fault(&mut self) -> bool {
        self.stepper.clear_fault()
    }
}
