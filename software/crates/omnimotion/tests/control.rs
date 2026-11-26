use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use omnimotion::{
    AbortState, CommandCompletion, CommandError, ControlPlane, JobProgress, LifecycleState,
    MachineCommand, RequestId, StandardCommand, StatusSnapshot, SubmitError,
};

type TestPlane = ControlPlane<NoopRawMutex, u16, 2>;

#[test]
fn command_capacity_and_request_ids_are_explicit() {
    let plane = TestPlane::new(StatusSnapshot::default());
    let (client, machine) = plane.split();

    let first = client
        .submit(MachineCommand::Standard(StandardCommand::Pause))
        .unwrap();
    let second = client.submit(MachineCommand::Application(7)).unwrap();
    assert_eq!(
        client.submit(MachineCommand::Standard(StandardCommand::Stop)),
        Err(SubmitError::QueueFull)
    );
    assert_ne!(first, second);
    assert_eq!(machine.try_next().unwrap().id, first);
    assert_eq!(machine.try_next().unwrap().id, second);
    assert!(machine.try_next().is_none());
}

#[test]
fn completions_remain_bounded_and_distinguish_old_requests() {
    let plane = TestPlane::new(StatusSnapshot::default());
    let (client, machine) = plane.split();
    let old = client
        .submit(MachineCommand::Standard(StandardCommand::Pause))
        .unwrap();
    let current = client
        .submit(MachineCommand::Standard(StandardCommand::Stop))
        .unwrap();
    machine.try_next();
    machine.try_next();

    pollster::block_on(machine.complete(CommandCompletion {
        id: old,
        result: Ok(()),
    }));
    pollster::block_on(machine.complete(CommandCompletion {
        id: current,
        result: Err(CommandError::InvalidState),
    }));

    assert_eq!(client.try_completion().unwrap().id, old);
    assert_eq!(
        client.try_completion(),
        Some(CommandCompletion {
            id: current,
            result: Err(CommandError::InvalidState),
        })
    );
}

#[test]
fn status_is_published_as_one_coherent_value() {
    let plane = TestPlane::new(StatusSnapshot::default());
    let (client, machine) = plane.split();
    let snapshot = StatusSnapshot {
        lifecycle: LifecycleState::Running,
        raw_position: [10, -20, 30],
        available_axes: 0b011,
        drivers_enabled: true,
        active_request: Some(RequestId(42)),
        abort: AbortState::default(),
        latched_fault: None,
        job: Some(JobProgress {
            completed: 4,
            total: 10,
        }),
    };
    machine.publish(snapshot);

    assert_eq!(client.status(), snapshot);
    assert_eq!(client.status().available_axes, 0b011);
    assert_eq!(client.status().raw_position, [10, -20, 30]);
}

#[test]
fn handles_copy_even_when_the_mutex_and_command_types_do_not() {
    struct NotCopy;
    let plane = ControlPlane::<NoopRawMutex, NotCopy, 2>::new(StatusSnapshot::default());
    let (client, machine) = plane.split();
    let (other_client, other_machine) = (client, machine);

    client.submit(MachineCommand::Application(NotCopy)).unwrap();
    other_client
        .submit(MachineCommand::Application(NotCopy))
        .unwrap();
    assert!(machine.try_next().is_some());
    assert!(other_machine.try_next().is_some());
}

#[test]
fn unavailable_m2_is_status_not_a_machine_fault() {
    let plane = TestPlane::new(StatusSnapshot {
        available_axes: 0b011,
        ..StatusSnapshot::default()
    });
    let (client, _) = plane.split();

    assert_eq!(client.status().available_axes, 0b011);
    assert_eq!(client.status().latched_fault, None);
}
