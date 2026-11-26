use crate::motion::planner::Segment;
use crate::{AxisMask, AXES};

/// What the pins should do this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    pub step: AxisMask,
    /// Axes travelling in the positive direction.
    pub dir: AxisMask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineError {
    InvalidSegment,
    CoordinateOverflow,
}

/// Turns segments into step pulses. An NCO accumulator advances by the segment
/// rate every tick; each overflow is one master step, which Bresenham
/// distributes across the axes. Positions are counted as steps happen, so they
/// stay exact even if a move is aborted.
#[derive(Clone, Copy, Debug)]
pub struct StepEngine {
    rate: u32,
    accumulator: u32,
    steps_left: u32,
    move_steps_left: u64,
    total: u64,
    delta: [u64; AXES],
    bresenham: [u64; AXES],
    dir: AxisMask,
    position: [i64; AXES],
}

impl Default for StepEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl StepEngine {
    pub const fn new() -> Self {
        Self {
            rate: 0,
            accumulator: 0,
            steps_left: 0,
            move_steps_left: 0,
            total: 0,
            delta: [0; AXES],
            bresenham: [0; AXES],
            dir: AxisMask::MAX,
            position: [0; AXES],
        }
    }

    pub fn idle(&self) -> bool {
        self.steps_left == 0
    }

    pub fn load(&mut self, segment: Segment) -> Result<(), EngineError> {
        if segment.rate == 0 || segment.steps == 0 {
            return Err(EngineError::InvalidSegment);
        }
        if let Some(start) = segment.start {
            self.total = start.delta.iter().copied().max().unwrap_or(0);
            if self.total == 0 || start.delta.iter().any(|delta| *delta > self.total) {
                return Err(EngineError::InvalidSegment);
            }
            self.delta = start.delta;
            self.dir = start.dir;
            // Half a master step of lead spreads each axis evenly rather than
            // bunching its steps against the start of the move.
            self.bresenham = [self.total / 2; AXES];
            self.accumulator = 0;
            self.move_steps_left = self.total;
        } else if self.total == 0 {
            return Err(EngineError::InvalidSegment);
        }
        if segment.steps as u64 > self.move_steps_left
            || (!segment.last && segment.steps as u64 == self.move_steps_left)
        {
            return Err(EngineError::InvalidSegment);
        }
        if segment.last {
            // A stopped move ends short of its target.
            self.move_steps_left = segment.steps as u64;
        }

        self.rate = segment.rate;
        self.steps_left = segment.steps;
        Ok(())
    }

    pub fn tick(&mut self) -> Result<Tick, EngineError> {
        let mut tick = Tick {
            step: 0,
            dir: self.dir,
        };

        if self.steps_left == 0 {
            return Ok(tick);
        }

        let (accumulator, overflowed) = self.accumulator.overflowing_add(self.rate);
        self.accumulator = accumulator;
        if !overflowed {
            return Ok(tick);
        }

        let mut next_bresenham = self.bresenham;
        let mut next_position = self.position;
        for axis in 0..AXES {
            let threshold = self.total - self.delta[axis];
            if self.bresenham[axis] >= threshold {
                next_bresenham[axis] = self.bresenham[axis] - threshold;
                tick.step |= 1 << axis;
                next_position[axis] = if self.dir & (1 << axis) != 0 {
                    self.position[axis].checked_add(1)
                } else {
                    self.position[axis].checked_sub(1)
                }
                .ok_or(EngineError::CoordinateOverflow)?;
            } else {
                next_bresenham[axis] = self.bresenham[axis] + self.delta[axis];
            }
        }

        self.steps_left -= 1;
        self.move_steps_left -= 1;
        self.bresenham = next_bresenham;
        self.position = next_position;
        Ok(tick)
    }

    pub fn position(&self) -> [i64; AXES] {
        self.position
    }

    pub fn set_position(&mut self, position: [i64; AXES]) {
        self.position = position;
    }

    pub fn abort(&mut self) {
        self.steps_left = 0;
        self.move_steps_left = 0;
        self.rate = 0;
    }
}
