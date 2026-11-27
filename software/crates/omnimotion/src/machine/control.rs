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

impl CommandError {
    pub fn name(self) -> &'static str {
        match self {
            Self::MachineBusy => "busy",
            Self::InvalidState => "invalid state",
            Self::UnavailableAxis => "axis unavailable",
            Self::FaultLatched => "fault latched",
            Self::InvalidCommand => "invalid command",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandCompletion {
    pub id: RequestId,
    pub result: Result<(), CommandError>,
    /// Why the command failed, for the operator, when the machine can say
    /// more than the `CommandError`.
    pub reason: Option<&'static str>,
}

/// Completions wait to be collected by request ID. Once this many wait, each
/// new one replaces the oldest.
pub const KEPT_COMPLETIONS: usize = 16;

/// The newest completions, oldest overwritten first.
struct Completions {
    slots: [Option<CommandCompletion>; KEPT_COMPLETIONS],
    next: usize,
}

impl Completions {
    const fn new() -> Self {
        Self {
            slots: [None; KEPT_COMPLETIONS],
            next: 0,
        }
    }

    fn insert(&mut self, completion: CommandCompletion) {
        self.slots[self.next] = Some(completion);
        self.next = (self.next + 1) % KEPT_COMPLETIONS;
    }

    fn take(&mut self, id: RequestId) -> Option<CommandCompletion> {
        self.slots
            .iter_mut()
            .find(|slot| slot.is_some_and(|completion| completion.id == id))
            .and_then(Option::take)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusSnapshot {
    pub lifecycle: LifecycleState,
    pub raw_position: [i64; AXES],
    /// The raw counters at each axis's zero: positions are `raw_position`
    /// minus these.
    pub origin: [i64; AXES],
    pub available_axes: AxisMask,
    pub drivers_enabled: bool,
    pub active_request: Option<RequestId>,
    pub abort: AbortState,
    pub latched_fault: Option<u32>,
    pub job: Option<JobProgress>,
}

/// A bounded cross-core command queue, the completions waiting to be
/// collected, and a coherent copied status snapshot.
///
/// Applications normally place this in static storage and call [`Self::split`]
/// once. Only the returned handles cross cores; neither contains a board
/// reference.
pub struct ControlPlane<M, A, const N: usize>
where
    M: RawMutex,
{
    commands: Channel<M, CommandRequest<A>, N>,
    completions: Mutex<M, RefCell<Completions>>,
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
            completions: Mutex::new(RefCell::new(Completions::new())),
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

    /// The completion of request `id`, once, if it has finished and is still kept.
    pub fn take_completion(&self, id: RequestId) -> Option<CommandCompletion> {
        self.plane
            .completions
            .lock(|completions| completions.borrow_mut().take(id))
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

    pub fn complete(&self, completion: CommandCompletion) {
        self.plane
            .completions
            .lock(|completions| completions.borrow_mut().insert(completion));
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
