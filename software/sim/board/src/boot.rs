use embassy_executor::Spawner;
use esp_hal::peripherals::{CPU_CTRL, SW_INTERRUPT, SYSTIMER};

/// The scheduler's own peripherals. Built by [`crate::split!`].
pub struct Rtos {
    #[doc(hidden)]
    pub systimer: SYSTIMER<'static>,
    #[doc(hidden)]
    pub software_interrupt: SW_INTERRUPT<'static>,
    #[doc(hidden)]
    pub cpu_control: CPU_CTRL<'static>,
}

/// Starts the second core's executor and returns with the calling core free for the application.
pub fn boot_on_second_core(rtos: Rtos, init: impl FnOnce(Spawner) + Send + 'static) {
    let _ = rtos;
    init(omnimotion_sim::exec::new_core());
}
