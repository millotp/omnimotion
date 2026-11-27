#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LifecycleState {
    #[default]
    Idle,
    Running,
    Pausing,
    Paused,
    Stopping,
    Completed,
    Faulted,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JobProgress {
    pub completed: u32,
    pub total: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleError {
    InvalidTransition,
    InvalidProgress,
    InterruptionPending,
    NotAtSafeBoundary,
}

/// Pure job state. The application performs motion and explicitly reports
/// operation boundaries and abort acknowledgement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lifecycle {
    state: LifecycleState,
    progress: Option<JobProgress>,
    safe_boundary: bool,
    interruption_acknowledged: bool,
}

impl Lifecycle {
    pub const fn new() -> Self {
        Self {
            state: LifecycleState::Idle,
            progress: None,
            safe_boundary: true,
            interruption_acknowledged: true,
        }
    }

    pub const fn state(&self) -> LifecycleState {
        self.state
    }

    pub const fn progress(&self) -> Option<JobProgress> {
        self.progress
    }

    pub const fn at_safe_boundary(&self) -> bool {
        self.safe_boundary
    }

    pub fn start(&mut self, total: u32) -> Result<(), LifecycleError> {
        if self.state != LifecycleState::Idle {
            return Err(LifecycleError::InvalidTransition);
        }
        if total == 0 {
            return Err(LifecycleError::InvalidProgress);
        }
        self.state = LifecycleState::Running;
        self.progress = Some(JobProgress {
            completed: 0,
            total,
        });
        self.safe_boundary = true;
        self.interruption_acknowledged = true;
        Ok(())
    }

    /// Starts a job from a checkpoint: running, with `progress.completed`
    /// operations already done.
    pub fn restore(&mut self, progress: JobProgress) -> Result<(), LifecycleError> {
        if progress.completed >= progress.total {
            return Err(LifecycleError::InvalidProgress);
        }
        self.start(progress.total)?;
        self.progress = Some(progress);
        Ok(())
    }

    /// While paused, takes the job back to `completed` operations, so the
    /// ones after it run again on resume.
    pub fn rewind(&mut self, completed: u32) -> Result<(), LifecycleError> {
        if self.state != LifecycleState::Paused {
            return Err(LifecycleError::InvalidTransition);
        }
        let progress = self
            .progress
            .as_mut()
            .ok_or(LifecycleError::InvalidProgress)?;
        if completed > progress.completed {
            return Err(LifecycleError::InvalidProgress);
        }
        progress.completed = completed;
        Ok(())
    }

    pub fn begin_operation(&mut self) -> Result<u32, LifecycleError> {
        if self.state != LifecycleState::Running || !self.safe_boundary {
            return Err(LifecycleError::InvalidTransition);
        }
        let progress = self.progress.ok_or(LifecycleError::InvalidProgress)?;
        if progress.completed >= progress.total {
            return Err(LifecycleError::InvalidProgress);
        }
        self.safe_boundary = false;
        Ok(progress.completed)
    }

    pub fn complete_operation(&mut self) -> Result<(), LifecycleError> {
        if self.state != LifecycleState::Running || self.safe_boundary {
            return Err(LifecycleError::InvalidTransition);
        }
        let progress = self
            .progress
            .as_mut()
            .ok_or(LifecycleError::InvalidProgress)?;
        progress.completed = progress
            .completed
            .checked_add(1)
            .ok_or(LifecycleError::InvalidProgress)?;
        self.safe_boundary = true;
        if progress.completed == progress.total {
            self.state = LifecycleState::Completed;
        }
        Ok(())
    }

    pub fn request_pause(&mut self) -> Result<bool, LifecycleError> {
        if self.state != LifecycleState::Running {
            return Err(LifecycleError::InvalidTransition);
        }
        self.state = LifecycleState::Pausing;
        self.interruption_acknowledged = self.safe_boundary;
        Ok(!self.safe_boundary)
    }

    pub fn acknowledge_interruption(&mut self) -> Result<(), LifecycleError> {
        if !matches!(
            self.state,
            LifecycleState::Pausing | LifecycleState::Stopping
        ) {
            return Err(LifecycleError::InvalidTransition);
        }
        self.interruption_acknowledged = true;
        Ok(())
    }

    /// Application declaration that recovery has returned the machine to the
    /// last completed-operation boundary. Progress is deliberately unchanged.
    pub fn declare_safe_boundary(&mut self) -> Result<(), LifecycleError> {
        if self.state != LifecycleState::Pausing {
            return Err(LifecycleError::InvalidTransition);
        }
        if !self.interruption_acknowledged {
            return Err(LifecycleError::InterruptionPending);
        }
        self.safe_boundary = true;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), LifecycleError> {
        if self.state != LifecycleState::Pausing {
            return Err(LifecycleError::InvalidTransition);
        }
        if !self.interruption_acknowledged {
            return Err(LifecycleError::InterruptionPending);
        }
        if !self.safe_boundary {
            return Err(LifecycleError::NotAtSafeBoundary);
        }
        self.state = LifecycleState::Paused;
        Ok(())
    }

    /// Pauses once the application has interrupted the running operation (if `request_pause` said one
    /// was) and taken the machine back to the last completed-operation boundary.
    pub fn pause_interrupted(&mut self) -> Result<(), LifecycleError> {
        self.acknowledge_interruption()?;
        self.declare_safe_boundary()?;
        self.pause()
    }

    pub fn resume(&mut self) -> Result<(), LifecycleError> {
        if self.state != LifecycleState::Paused {
            return Err(LifecycleError::InvalidTransition);
        }
        self.state = LifecycleState::Running;
        Ok(())
    }

    pub fn request_stop(&mut self) -> Result<bool, LifecycleError> {
        let needs_interruption = match self.state {
            LifecycleState::Running => !self.safe_boundary,
            LifecycleState::Pausing => !self.interruption_acknowledged,
            LifecycleState::Paused => false,
            _ => return Err(LifecycleError::InvalidTransition),
        };
        self.state = LifecycleState::Stopping;
        self.interruption_acknowledged = !needs_interruption;
        Ok(needs_interruption)
    }

    pub fn acknowledge_stop(&mut self) -> Result<(), LifecycleError> {
        if self.state != LifecycleState::Stopping {
            return Err(LifecycleError::InvalidTransition);
        }
        if !self.interruption_acknowledged {
            return Err(LifecycleError::InterruptionPending);
        }
        self.reset_to_idle();
        Ok(())
    }

    /// Stops once the application has interrupted the running operation, if `request_stop` said one was.
    pub fn stop_interrupted(&mut self) -> Result<(), LifecycleError> {
        self.acknowledge_interruption()?;
        self.acknowledge_stop()
    }

    pub fn fault(&mut self) {
        self.state = LifecycleState::Faulted;
        self.safe_boundary = false;
        self.interruption_acknowledged = false;
    }

    pub fn recover_fault(&mut self) -> Result<(), LifecycleError> {
        if self.state != LifecycleState::Faulted {
            return Err(LifecycleError::InvalidTransition);
        }
        self.reset_to_idle();
        Ok(())
    }

    pub fn reset_completed(&mut self) -> Result<(), LifecycleError> {
        if self.state != LifecycleState::Completed {
            return Err(LifecycleError::InvalidTransition);
        }
        self.reset_to_idle();
        Ok(())
    }

    fn reset_to_idle(&mut self) {
        self.state = LifecycleState::Idle;
        self.progress = None;
        self.safe_boundary = true;
        self.interruption_acknowledged = true;
    }
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self::new()
    }
}
