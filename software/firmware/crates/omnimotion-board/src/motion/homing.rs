use core::cell::Cell;

use embassy_futures::select::{select, Either};
use embassy_time::{Duration, Timer};
use omnimotion::devices::endstop::{EndstopKind, EndstopRole};
use omnimotion::devices::tmc2209::{tstep_for_rate, DriverConfig, Error};
use omnimotion::{Limits, AXES};

use crate::motion::stepper;
use crate::motion::supervisor::{Fault, FaultSupervisor, HomingTrigger};
use crate::motion::{Motion, MotionError, Outcome};
use crate::{TmcDriver, TmcError};

/// The driver's stall flag may still be set from the previous move, so nothing
/// may move for this long before a StallGuard pass starts.
const SETTLE: Duration = Duration::from_secs(2);

/// What ends a homing pass.
#[derive(Clone, Copy, Debug)]
pub enum HomingSensor {
    /// Driver load measurement on DIAG. Needs a hard end of travel to push
    /// against, so it cannot home an axis that turns without limit.
    StallGuard {
        /// Reduced current improves StallGuard sensitivity.
        current_ma: f32,
        /// 255 is most sensitive, 0 least.
        threshold: u8,
    },
    /// A switch or hall sensor on one of J11-J16, wired as
    /// `EndstopRole::Reference`.
    Endstop { endstop: usize },
}

/// Distances and rates are in the axis's own units, as for every other move.
#[derive(Clone, Copy, Debug)]
pub struct HomingConfig {
    pub sensor: HomingSensor,
    /// Signed travel budget; its sign is the direction of the pass.
    pub travel: f32,
    /// Units per second.
    pub rate: f32,
    /// Units per second squared.
    pub accel: f32,
    /// Distance to back off the stop, opposite the travel direction. The axis
    /// zeroes here, not on the stop.
    pub backoff: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Homed {
    /// Reached the stop and the axis is now zeroed.
    Found,
    /// Ran the whole travel budget without the sensor triggering. The outputs
    /// are left disabled; `faults.enable_drivers()` turns them back on.
    NotFound,
}

#[derive(Debug)]
pub enum HomingError {
    Driver(Error<TmcError>),
    Motion(MotionError),
    PreassertedDiag,
    /// Began sitting on its reference. Nothing moved and no fault was latched.
    PreassertedEndstop,
    DriverFault,
    Cleanup,
    Backoff,
    /// The reference was still asserted after backing off, so the axis was not
    /// zeroed. Nothing was latched; increase `backoff`.
    BackoffTooShort,
    /// Interrupted by a fault the supervisor latched, or by an abort request.
    Aborted,
    InvalidConfiguration,
}

impl From<Error<TmcError>> for HomingError {
    fn from(error: Error<TmcError>) -> Self {
        Self::Driver(error)
    }
}

impl From<MotionError> for HomingError {
    fn from(error: MotionError) -> Self {
        Self::Motion(error)
    }
}

async fn fail_safe(
    faults: &mut FaultSupervisor,
    fault: Fault,
    error: HomingError,
) -> Result<Homed, HomingError> {
    faults.trip(fault).await;
    Err(error)
}

async fn restore(driver: &mut TmcDriver<'_>, run_config: &DriverConfig, armed: bool) -> bool {
    if !armed {
        return true;
    }
    let config = driver.apply(run_config).await;
    let stall = driver.set_stallguard_threshold(0).await;
    let velocity_gate = driver.set_coolstep_threshold(0).await;
    config.is_ok() && stall.is_ok() && velocity_gate.is_ok()
}

async fn arm_stallguard(
    driver: &mut TmcDriver<'_>,
    rate: f32,
    current_ma: f32,
    threshold: u8,
    microsteps: u32,
) -> Result<(), Error<TmcError>> {
    // Hold current must equal run current for the whole pass. Changing current
    // mid-homing makes StallGuard erratic.
    driver.set_current(current_ma, current_ma).await?;
    // StallGuard4 only measures in StealthChop, so the pass forces it at every
    // velocity. `restore` reapplies the normal run configuration afterwards.
    driver.set_spreadcycle(false).await?;
    driver.set_stealthchop_threshold(0).await?;
    driver.set_stallguard_threshold(threshold).await?;
    // StallGuard is velocity gated. Arming from half the homing rate keeps the
    // ramp from tripping it early.
    let coolstep_threshold =
        tstep_for_rate(rate / 2.0, microsteps).map_err(Error::InvalidConfiguration)?;
    driver.set_coolstep_threshold(coolstep_threshold).await?;
    Ok(())
}

pub async fn home(
    motion: &mut Motion,
    driver: &mut TmcDriver<'_>,
    faults: &mut FaultSupervisor,
    axis: usize,
    config: &HomingConfig,
    run_config: &DriverConfig,
) -> Result<Homed, HomingError> {
    let axis_config = motion.config(axis)?;
    let travel_steps = axis_config
        .steps(config.travel)
        .map_err(MotionError::from)?;
    let backoff_steps = axis_config
        .steps(config.backoff)
        .map_err(MotionError::from)?;
    let limits = Limits {
        max_rate: config.rate * axis_config.steps_per_unit,
        min_rate: config.rate * axis_config.steps_per_unit / 4.0,
        accel: config.accel * axis_config.steps_per_unit,
    };
    if travel_steps == 0 || backoff_steps <= 0 || limits.validate().is_err() {
        return Err(HomingError::InvalidConfiguration);
    }

    let trigger = match config.sensor {
        HomingSensor::StallGuard { .. } => HomingTrigger::Diag(axis),
        HomingSensor::Endstop { endstop } => {
            // A limit would latch Fault::Endstop the instant the pass worked.
            let Some(wiring) = faults.endstop_config(endstop) else {
                return Err(HomingError::InvalidConfiguration);
            };
            if wiring.kind == EndstopKind::Unused || wiring.role != EndstopRole::Reference {
                return Err(HomingError::InvalidConfiguration);
            }
            // Nothing has moved yet, so refuse without latching.
            if faults.endstop_asserted(endstop) {
                return Err(HomingError::PreassertedEndstop);
            }
            HomingTrigger::Endstop(endstop)
        }
    };
    let stallguard = matches!(config.sensor, HomingSensor::StallGuard { .. });

    if faults.diag_asserted(axis) {
        return fail_safe(faults, Fault::Diag, HomingError::PreassertedDiag).await;
    }

    if let HomingSensor::StallGuard {
        current_ma,
        threshold,
    } = config.sensor
    {
        let microsteps = run_config.microsteps.number_of_microsteps();
        if let Err(error) =
            arm_stallguard(driver, limits.max_rate, current_ma, threshold, microsteps).await
        {
            restore(driver, run_config, stallguard).await;
            return fail_safe(faults, Fault::Driver, HomingError::Driver(error)).await;
        }

        match select(Timer::after(SETTLE), faults.wait_for_homing_event(trigger)).await {
            Either::First(()) => {}
            Either::Second(None) => {
                restore(driver, run_config, stallguard).await;
                return fail_safe(faults, Fault::Diag, HomingError::PreassertedDiag).await;
            }
            Either::Second(Some(_)) => {
                if !restore(driver, run_config, stallguard).await {
                    return Err(HomingError::Cleanup);
                }
                return Err(HomingError::Aborted);
            }
        }
    }

    let mut travel = [0i64; AXES];
    travel[axis] = travel_steps;

    // A stall must stop at once, as the motor is pushing the frame. A sensor
    // trip ramps down instead, so a heavy axis keeps its steps, and the
    // backoff is measured from where the sensor tripped.
    let tripped_at = Cell::new(None);
    let aborts = stepper::abort_generation();
    let watch = async {
        let event = faults.wait_for_homing_event(trigger).await;
        if event.is_some() {
            return event;
        }
        if stallguard {
            let generation = stepper::abort();
            stepper::await_abort(generation).await;
            tripped_at.set(Some(stepper::position()[axis]));
            return None;
        }
        tripped_at.set(Some(stepper::position()[axis]));
        stepper::request_stop();
        Some(faults.wait_for_fault().await)
    };
    let ended = select(motion.move_by_with(travel, limits), watch).await;
    let interrupted = stepper::abort_generation() != aborts && !stallguard;
    let triggered = match (ended, tripped_at.get()) {
        (Either::First(Err(error)), _) => {
            restore(driver, run_config, stallguard).await;
            return fail_safe(faults, Fault::MotionEngine, HomingError::Motion(error)).await;
        }
        (Either::First(Ok(_)), Some(_)) | (Either::Second(None), _) if !interrupted => true,
        (Either::First(Ok(Outcome::Completed)), None) => false,
        _ => {
            if !restore(driver, run_config, stallguard).await {
                return Err(HomingError::Cleanup);
            }
            return Err(HomingError::Aborted);
        }
    };

    // A stall only reports load, so ask the driver whether that load was the
    // frame or a real fault.
    if triggered && stallguard {
        let severe = match driver_is_faulted(driver).await {
            Ok(severe) => severe,
            Err(error) => {
                restore(driver, run_config, stallguard).await;
                return fail_safe(faults, Fault::Driver, HomingError::Driver(error)).await;
            }
        };
        if severe {
            restore(driver, run_config, stallguard).await;
            return fail_safe(faults, Fault::Driver, HomingError::DriverFault).await;
        }
    }

    if !restore(driver, run_config, stallguard).await {
        return fail_safe(faults, Fault::Driver, HomingError::Cleanup).await;
    }

    if !triggered {
        faults.disable_drivers();
        return Ok(Homed::NotFound);
    }

    // Come off the stop straight away, or the motor keeps pushing the frame.
    let trip = tripped_at.get().unwrap_or(stepper::position()[axis]);
    let mut backoff = [0i64; AXES];
    backoff[axis] = trip - backoff_steps * travel_steps.signum() - stepper::position()[axis];
    let backed_off = match select(
        motion.move_by_with(backoff, limits),
        faults.wait_for_backoff_fault(trigger),
    )
    .await
    {
        Either::First(result) => result,
        Either::Second(_) => return Err(HomingError::Aborted),
    };
    match backed_off {
        Ok(Outcome::Completed) => {}
        Ok(Outcome::Aborted) => return Err(HomingError::Aborted),
        Err(_) => return fail_safe(faults, Fault::MotionEngine, HomingError::Backoff).await,
    }
    if faults.diag_asserted(axis) {
        return fail_safe(faults, Fault::Diag, HomingError::Backoff).await;
    }
    if let HomingTrigger::Endstop(endstop) = trigger {
        if faults.endstop_asserted(endstop) {
            return Err(HomingError::BackoffTooShort);
        }
    }

    motion.zero(axis)?;
    Ok(Homed::Found)
}

async fn driver_is_faulted(driver: &mut TmcDriver<'_>) -> Result<bool, Error<TmcError>> {
    let global = driver.global_status().await?;
    let status = driver.status().await?;
    Ok(global.drv_err()
        || global.uv_cp()
        || status.ot()
        || status.s2ga()
        || status.s2gb()
        || status.s2vsa()
        || status.s2vsb())
}
