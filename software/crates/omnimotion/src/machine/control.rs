use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::channel::Channel;

use crate::{AbortState, AxisMask, JobProgress, LifecycleState, AXES};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequestId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardCommand {
    Home { axis: usize },
    Pause,
    Resume,
    Stop,
    EnableDrivers,
    DisableDrivers,
    ClearFault,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MachineCommand<A = ()> {
    Standard(StandardCommand),
    Application(A),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandRequest<A = ()> {
    pub id: RequestId,
    pub command: MachineCommand<A>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError {
    QueueFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandError {
    MachineBusy,
    InvalidState,
    UnavailableAxis,
    FaultLatched,
    InvalidCommand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandCompletion {
    pub id: RequestId,
    pub result: Result<(), CommandError>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusSnapshot {
    pub lifecycle: LifecycleState,
    pub raw_position: [i64; AXES],
    pub available_axes: AxisMask,
    pub drivers_enabled: bool,
    pub active_request: Option<RequestId>,
    pub abort: AbortState,
    pub latched_fault: Option<u32>,
    pub job: Option<JobProgress>,
}

/// Bounded cross-core queues and a coherent copied status snapshot.
///
/// Applications normally place this in static storage and call [`Self::split`]
/// once. Only the returned handles cross cores; neither contains a board
/// reference.
pub struct ControlPlane<M, A, const N: usize>
where
    M: RawMutex,
{
    commands: Channel<M, CommandRequest<A>, N>,
    completions: Channel<M, CommandCompletion, N>,
    status: Mutex<M, RefCell<StatusSnapshot>>,
    next_request: AtomicU32,
}

impl<M, A, const N: usize> ControlPlane<M, A, N>
where
    M: RawMutex,
{
    pub const fn new(initial_status: StatusSnapshot) -> Self {
        Self {
            commands: Channel::new(),
            completions: Channel::new(),
            status: Mutex::new(RefCell::new(initial_status)),
            next_request: AtomicU32::new(0),
        }
    }

    pub fn split(&self) -> (ControlClient<'_, M, A, N>, MachineControl<'_, M, A, N>) {
        (
            ControlClient { plane: self },
            MachineControl { plane: self },
        )
    }
}

pub struct ControlClient<'a, M, A, const N: usize>
where
    M: RawMutex,
{
    plane: &'a ControlPlane<M, A, N>,
}

impl<M: RawMutex, A, const N: usize> Clone for ControlClient<'_, M, A, N> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M: RawMutex, A, const N: usize> Copy for ControlClient<'_, M, A, N> {}

impl<M, A, const N: usize> ControlClient<'_, M, A, N>
where
    M: RawMutex,
{
    pub fn submit(&self, command: MachineCommand<A>) -> Result<RequestId, SubmitError> {
        let id = RequestId(
            self.plane
                .next_request
                .fetch_add(1, Ordering::AcqRel)
                .wrapping_add(1),
        );
        self.plane
            .commands
            .try_send(CommandRequest { id, command })
            .map_err(|_| SubmitError::QueueFull)?;
        Ok(id)
    }

    pub fn status(&self) -> StatusSnapshot {
        self.plane.status.lock(|status| *status.borrow())
    }

    pub fn try_completion(&self) -> Option<CommandCompletion> {
        self.plane.completions.try_receive().ok()
    }

    pub async fn completion(&self) -> CommandCompletion {
        self.plane.completions.receive().await
    }
}

pub struct MachineControl<'a, M, A, const N: usize>
where
    M: RawMutex,
{
    plane: &'a ControlPlane<M, A, N>,
}

impl<M: RawMutex, A, const N: usize> Clone for MachineControl<'_, M, A, N> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M: RawMutex, A, const N: usize> Copy for MachineControl<'_, M, A, N> {}

impl<M, A, const N: usize> MachineControl<'_, M, A, N>
where
    M: RawMutex,
{
    pub fn try_next(&self) -> Option<CommandRequest<A>> {
        self.plane.commands.try_receive().ok()
    }

    pub async fn next(&self) -> CommandRequest<A> {
        self.plane.commands.receive().await
    }

    /// Waits for bounded completion capacity instead of dropping an accepted
    /// command's result.
    pub async fn complete(&self, completion: CommandCompletion) {
        self.plane.completions.send(completion).await;
    }

    pub fn publish(&self, snapshot: StatusSnapshot) {
        self.plane
            .status
            .lock(|status| *status.borrow_mut() = snapshot);
    }
}

#[cfg(test)]
mod tests {
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;

    use super::*;

    #[test]
    fn request_ids_wrap_without_reusing_the_previous_id() {
        let plane = ControlPlane::<NoopRawMutex, (), 2>::new(StatusSnapshot::default());
        plane.next_request.store(u32::MAX - 1, Ordering::Release);
        let (client, _) = plane.split();

        assert_eq!(
            client.submit(MachineCommand::Application(())).unwrap(),
            RequestId(u32::MAX)
        );
        assert_eq!(
            client.submit(MachineCommand::Application(())).unwrap(),
            RequestId(0)
        );
    }
}
