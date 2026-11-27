//! Asking the drivers how they are while the machine runs.

use omnimotion::devices::tmc2209::{DriverProblem, DriverStatus};
use omnimotion::AXES;

use crate::motion::supervisor::Fault;
use crate::Board;

/// What a poll of the drivers found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverHealth {
    /// `DriverStatus::UNKNOWN` for an axis that is not available or did not
    /// answer.
    pub status: [DriverStatus; AXES],
    /// The first axis with a problem, and the problem. The poll has already
    /// tripped `Fault::Driver` for it.
    pub problem: Option<(usize, DriverProblem)>,
}

impl Board {
    /// Reads every available axis's driver status, asking twice before
    /// calling a driver silent. Overheating (warning included) and shorts trip
    /// `Fault::Driver`;
    /// so does silence with `require_response`, as while a job runs, since at
    /// idle a driver may only lack motor power.
    pub async fn poll_drivers(&mut self, require_response: bool) -> DriverHealth {
        let mut status = [DriverStatus::UNKNOWN; AXES];
        for (axis, status) in status.iter_mut().enumerate() {
            if !self.motion.axis_available(axis) {
                continue;
            }
            for _attempt in 0..2 {
                if let Some(read) = self.read_driver(axis).await {
                    *status = read;
                    break;
                }
            }
        }
        let problem = (0..AXES)
            .filter(|&axis| self.motion.axis_available(axis))
            .find_map(|axis| {
                status[axis]
                    .problem()
                    .filter(|problem| require_response || *problem != DriverProblem::Silent)
                    .map(|problem| (axis, problem))
            });
        if problem.is_some() {
            self.faults.trip(Fault::Driver).await;
        }
        DriverHealth { status, problem }
    }

    async fn read_driver(&mut self, axis: usize) -> Option<DriverStatus> {
        let mut driver = self.driver(axis)?;
        let version = driver.version().await.ok()?;
        let status = driver.status().await.ok()?;
        Some(DriverStatus::from_registers(version, status))
    }
}
