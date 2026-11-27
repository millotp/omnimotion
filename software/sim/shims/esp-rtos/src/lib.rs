//! esp-rtos' embassy executor, as one more simulated core.

pub mod embassy {
    use embassy_executor::Spawner;

    pub struct Executor(());

    impl Executor {
        #[allow(clippy::new_without_default)]
        pub fn new() -> Self {
            Self(())
        }

        /// Starts the executor's tasks, then hands the host thread back to the simulator for good.
        pub fn run(&'static mut self, init: impl FnOnce(Spawner)) -> ! {
            init(omnimotion_sim::exec::new_core());
            omnimotion_sim::park()
        }
    }
}
