use omnimotion::{JobProgress, Lifecycle, LifecycleError, LifecycleState, WriteGate};

fn persistence_allowed(job: &Lifecycle) -> bool {
    WriteGate {
        motion_drained: true,
        abort_pending: false,
        lifecycle: job.state(),
        safe_boundary: job.at_safe_boundary(),
    }
    .check()
    .is_ok()
}

#[test]
fn default_is_the_same_idle_state_as_new() {
    assert_eq!(Lifecycle::default(), Lifecycle::new());
    assert!(persistence_allowed(&Lifecycle::default()));
}

#[test]
fn progress_advances_only_for_complete_operations() {
    let mut job = Lifecycle::new();
    job.start(2).unwrap();
    assert_eq!(job.begin_operation(), Ok(0));
    assert_eq!(
        job.begin_operation(),
        Err(LifecycleError::InvalidTransition)
    );
    assert_eq!(
        job.progress(),
        Some(JobProgress {
            completed: 0,
            total: 2
        })
    );

    job.complete_operation().unwrap();
    assert_eq!(job.begin_operation(), Ok(1));
    job.complete_operation().unwrap();
    assert_eq!(job.state(), LifecycleState::Completed);
    assert_eq!(
        job.progress(),
        Some(JobProgress {
            completed: 2,
            total: 2
        })
    );
}

#[test]
fn immediate_pause_requires_abort_ack_and_a_safe_boundary() {
    let mut job = Lifecycle::new();
    job.start(3).unwrap();
    job.begin_operation().unwrap();
    assert_eq!(job.request_pause(), Ok(true));
    assert_eq!(job.state(), LifecycleState::Pausing);
    assert_eq!(job.pause(), Err(LifecycleError::InterruptionPending));
    assert_eq!(
        job.declare_safe_boundary(),
        Err(LifecycleError::InterruptionPending)
    );

    job.acknowledge_interruption().unwrap();
    assert_eq!(job.pause(), Err(LifecycleError::NotAtSafeBoundary));
    job.declare_safe_boundary().unwrap();
    job.pause().unwrap();
    assert_eq!(job.state(), LifecycleState::Paused);
    assert_eq!(job.progress().unwrap().completed, 0);

    job.resume().unwrap();
    assert_eq!(job.begin_operation(), Ok(0));
}

#[test]
fn boundary_pause_needs_no_abort() {
    let mut job = Lifecycle::new();
    job.start(1).unwrap();
    assert_eq!(job.request_pause(), Ok(false));
    job.pause().unwrap();
    assert!(persistence_allowed(&job));
}

#[test]
fn stop_discards_resumable_progress() {
    let mut job = Lifecycle::new();
    job.start(2).unwrap();
    job.begin_operation().unwrap();
    job.complete_operation().unwrap();
    assert_eq!(job.request_stop(), Ok(false));
    job.acknowledge_stop().unwrap();

    assert_eq!(job.state(), LifecycleState::Idle);
    assert_eq!(job.progress(), None);
}

#[test]
fn an_active_stop_waits_for_abort_acknowledgement() {
    let mut job = Lifecycle::new();
    job.start(2).unwrap();
    job.begin_operation().unwrap();
    assert_eq!(job.request_stop(), Ok(true));
    assert_eq!(
        job.acknowledge_stop(),
        Err(LifecycleError::InterruptionPending)
    );
    job.acknowledge_interruption().unwrap();
    job.acknowledge_stop().unwrap();
    assert_eq!(job.state(), LifecycleState::Idle);
}

#[test]
fn an_interrupted_pause_or_stop_completes_in_one_call() {
    for interrupted in [false, true] {
        let mut job = Lifecycle::new();
        job.start(2).unwrap();
        if interrupted {
            job.begin_operation().unwrap();
        }
        assert_eq!(job.request_pause(), Ok(interrupted));
        job.pause_interrupted().unwrap();
        assert_eq!(job.state(), LifecycleState::Paused);
        assert_eq!(job.progress().unwrap().completed, 0);

        job.resume().unwrap();
        if interrupted {
            job.begin_operation().unwrap();
        }
        assert_eq!(job.request_stop(), Ok(interrupted));
        job.stop_interrupted().unwrap();
        assert_eq!(job.state(), LifecycleState::Idle);
    }
}

#[test]
fn faults_cannot_be_resumed_and_recovery_clears_progress() {
    let mut job = Lifecycle::new();
    job.start(2).unwrap();
    job.begin_operation().unwrap();
    job.fault();
    assert_eq!(job.state(), LifecycleState::Faulted);
    assert_eq!(job.resume(), Err(LifecycleError::InvalidTransition));
    assert!(!persistence_allowed(&job));

    job.recover_fault().unwrap();
    assert_eq!(job.state(), LifecycleState::Idle);
    assert_eq!(job.progress(), None);
}

#[test]
fn a_completed_job_resets_to_idle_and_can_start_again() {
    let mut job = Lifecycle::new();
    job.start(1).unwrap();
    assert_eq!(
        job.reset_completed(),
        Err(LifecycleError::InvalidTransition)
    );

    job.begin_operation().unwrap();
    job.complete_operation().unwrap();
    assert_eq!(job.state(), LifecycleState::Completed);

    job.reset_completed().unwrap();
    assert_eq!(job.state(), LifecycleState::Idle);
    assert_eq!(job.progress(), None);
    assert!(persistence_allowed(&job));
    job.start(3).unwrap();
}

#[test]
fn a_restored_job_runs_from_its_checkpoint() {
    let mut job = Lifecycle::new();
    let checkpoint = JobProgress {
        completed: 3,
        total: 5,
    };
    job.restore(checkpoint).unwrap();
    assert_eq!(job.state(), LifecycleState::Running);
    assert_eq!(job.progress(), Some(checkpoint));
    assert_eq!(job.begin_operation(), Ok(3));
    assert_eq!(
        job.restore(checkpoint),
        Err(LifecycleError::InvalidTransition)
    );

    let finished = JobProgress {
        completed: 5,
        total: 5,
    };
    assert_eq!(
        Lifecycle::new().restore(finished),
        Err(LifecycleError::InvalidProgress)
    );
}

#[test]
fn a_paused_job_rewinds_but_never_forward() {
    let mut job = Lifecycle::new();
    job.start(4).unwrap();
    for _ in 0..3 {
        job.begin_operation().unwrap();
        job.complete_operation().unwrap();
    }
    assert_eq!(job.rewind(1), Err(LifecycleError::InvalidTransition));
    job.request_pause().unwrap();
    job.pause().unwrap();
    assert_eq!(job.rewind(4), Err(LifecycleError::InvalidProgress));
    job.rewind(1).unwrap();
    job.resume().unwrap();
    assert_eq!(job.begin_operation(), Ok(1));
}

#[test]
fn invalid_transitions_are_rejected() {
    let mut job = Lifecycle::new();
    assert_eq!(job.start(0), Err(LifecycleError::InvalidProgress));
    assert_eq!(job.resume(), Err(LifecycleError::InvalidTransition));
    assert_eq!(
        job.complete_operation(),
        Err(LifecycleError::InvalidTransition)
    );
    assert_eq!(
        job.acknowledge_stop(),
        Err(LifecycleError::InvalidTransition)
    );

    job.start(1).unwrap();
    assert_eq!(job.start(1), Err(LifecycleError::InvalidTransition));
}
