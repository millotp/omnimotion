//! A hybrid stepper on a TMC2209 turning its load: torque from the coil current and the rotor's angle to the
//! driver's field, the supply and back-EMF limits, damping, drag with stiction and optional hard stops.
//! Missed steps are counted in whole electrical cycles.

use std::f64::consts::{FRAC_PI_2, PI, SQRT_2, TAU};

use serde::{Deserialize, Serialize};

use crate::tmc::Chip;

/// Damping ratio of the rotor about the field (mechanical losses, eddy currents, chopper).
const ZETA: f64 = 0.15;
/// Breakaway friction, as a multiple of the running drag.
const STICTION: f64 = 1.2;
/// Time constant of the estimate of the field's speed from its steps, s.
const FIELD_SPEED_TAU_S: f64 = 2e-3;

/// The motor's datasheet figures, and the supply it runs on.
#[derive(Clone, Debug, Deserialize)]
pub struct Motor {
    pub rated_a: f64,
    pub r_ohm: f64,
    pub l_mh: f64,
    pub hold_nm: f64,
    pub pole_pairs: f64,
    pub supply_v: f64,
    /// Fraction of the supply the chopper can put across the winding.
    pub headroom: f64,
}

impl Motor {
    /// Holding torque per amp of phase peak: both phases at rated current give the rated holding torque.
    pub fn torque_constant(&self) -> f64 {
        self.hold_nm / (SQRT_2 * self.rated_a)
    }

    /// Peak phase current the supply can still push through the winding at shaft speed `w` (rad/s).
    pub fn current_limit(&self, w: f64) -> f64 {
        let k = self.torque_constant();
        let we = w.abs() * self.pole_pairs;
        let (r, l, v) = (self.r_ohm, self.l_mh * 1e-3, self.supply_v * self.headroom);
        let a = r * r + (we * l).powi(2);
        let b = 2.0 * r * k * w.abs();
        let c = (k * w).powi(2) - v * v;
        if c >= 0.0 {
            return 0.0;
        }
        (-b + (b * b - 4.0 * a * c).sqrt()) / (2.0 * a)
    }
}

/// What the shaft turns, seen at the shaft.
#[derive(Clone, Copy, Debug)]
pub struct Load {
    /// Inertia, kg m², rotor included.
    pub j: f64,
    /// Coulomb drag, N m.
    pub drag_nm: f64,
    /// Hard stops on the shaft angle, rad.
    pub stops: Option<(f64, f64)>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Rotor {
    /// Shaft angle, rad.
    pub phi: f64,
    pub omega: f64,
    field_omega: f64,
    /// Electrical angle of the field at the last step.
    last_field: Option<f64>,
    /// The driver's power-on count at the last step: a new one means its sequencer restarted.
    power_ons: u32,
    /// Electrical cycle offset between rotor and field since the driver was energised.
    reference: Option<i64>,
    /// Full steps the rotor has fallen behind (or run ahead of) the field, in whole electrical cycles.
    pub slipped: i64,
}

impl Rotor {
    pub fn new(phi: f64) -> Self {
        Self {
            phi,
            omega: 0.0,
            field_omega: 0.0,
            last_field: None,
            power_ons: 0,
            reference: None,
            slipped: 0,
        }
    }

    /// How far the rotor trails `chip`'s field, in full steps (within ±2; negative when it runs ahead).
    pub fn lag_full_steps(&self, motor: &Motor, chip: &Chip) -> f64 {
        let delta = motor.pole_pairs * self.phi - chip.electrical_angle();
        ((delta + PI).rem_euclid(TAU) - PI) / FRAC_PI_2
    }

    /// Advances `dt` seconds behind `chip`'s field at phase current `i_rms`. Returns the full steps slipped,
    /// if the rotor just fell a whole electrical cycle behind (or ahead).
    pub fn step(
        &mut self,
        motor: &Motor,
        chip: &Chip,
        i_rms: f64,
        load: &Load,
        dt: f64,
    ) -> Option<i64> {
        let field = chip.electrical_angle();
        let restarted = std::mem::replace(&mut self.power_ons, chip.power_ons) != chip.power_ons;
        self.integrate(motor, field, restarted, i_rms, load, dt);
        self.track_slip(motor, field, i_rms)
    }

    /// Magnetic torque toward the field, damping about the field's speed, Coulomb drag with stiction, and
    /// the hard stops. `restarted` says the driver's sequencer went back to its start (power-on): its jump is
    /// not motion.
    fn integrate(
        &mut self,
        motor: &Motor,
        field: f64,
        restarted: bool,
        i_rms: f64,
        load: &Load,
        dt: f64,
    ) {
        let p = motor.pole_pairs;
        let raw = match self.last_field {
            Some(last) if !restarted => (field - last) / p / dt,
            _ => 0.0,
        };
        self.last_field = Some(field);
        self.field_omega += (raw - self.field_omega) * (dt / FIELD_SPEED_TAU_S).min(1.0);

        let i_peak = (i_rms * SQRT_2).min(motor.current_limit(self.omega));
        let k = motor.torque_constant() * i_peak;
        let delta = p * self.phi - field;
        let mut torque = -k * delta.sin();
        let stiffness = (k * p).max(1e-6);
        torque -= 2.0 * ZETA * (stiffness * load.j).sqrt() * (self.omega - self.field_omega);

        let drag = load.drag_nm;
        if self.omega.abs() < 1e-3 {
            if torque.abs() <= drag * STICTION {
                self.omega = 0.0;
                return;
            }
            torque -= drag * torque.signum();
        } else {
            torque -= drag * self.omega.signum();
        }
        let before = self.omega;
        self.omega += torque / load.j * dt;
        if before != 0.0 && before.signum() != self.omega.signum() && (torque.abs() < drag * 2.0) {
            self.omega = 0.0;
        }
        self.phi += self.omega * dt;
        if let Some((lo, hi)) = load.stops {
            if self.phi < lo {
                self.phi = lo;
                self.omega = self.omega.max(0.0);
            } else if self.phi > hi {
                self.phi = hi;
                self.omega = self.omega.min(0.0);
            }
        }
    }

    /// Follows the rotor's electrical cycle against the field.
    fn track_slip(&mut self, motor: &Motor, field: f64, current: f64) -> Option<i64> {
        if current == 0.0 {
            self.reference = None;
            return None;
        }
        let cycles = ((motor.pole_pairs * self.phi - field) / TAU).round() as i64;
        match self.reference.replace(cycles) {
            Some(r) if r != cycles => {
                let n = (cycles - r) * 4;
                self.slipped += n;
                Some(n)
            }
            _ => None,
        }
    }
}
