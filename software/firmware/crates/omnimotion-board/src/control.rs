use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use omnimotion::{
    ControlClient, ControlPlane, JobProgress, LifecycleState, MachineControl, RequestId,
    StatusSnapshot,
};

use crate::motion::supervisor;
use crate::Board;

pub const DEFAULT_QUEUE_DEPTH: usize = 8;

pub type BoardControlPlane<A, const N: usize = DEFAULT_QUEUE_DEPTH> =
    ControlPlane<CriticalSectionRawMutex, A, N>;
pub type BoardControlClient<'a, A, const N: usize = DEFAULT_QUEUE_DEPTH> =
    ControlClient<'a, CriticalSectionRawMutex, A, N>;
pub type BoardMachineControl<'a, A, const N: usize = DEFAULT_QUEUE_DEPTH> =
    MachineControl<'a, CriticalSectionRawMutex, A, N>;

/// Copies board state for publication without lending the board to readers.
pub fn snapshot(
    board: &Board,
    lifecycle: LifecycleState,
    active_request: Option<RequestId>,
    job: Option<JobProgress>,
) -> StatusSnapshot {
    let fault = supervisor::latched();
    StatusSnapshot {
        lifecycle,
        raw_position: board.motion.raw_position(),
        available_axes: board.motion.available_axes(),
        drivers_enabled: board.faults.drivers_enabled(),
        active_request,
        abort: board.motion.abort_handle().state(),
        latched_fault: (fault != 0).then_some(fault),
        job,
    }
}
