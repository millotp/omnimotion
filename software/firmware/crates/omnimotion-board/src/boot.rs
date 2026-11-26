use embassy_executor::Spawner;
use esp_hal::interrupt::software::{SoftwareInterrupt, SoftwareInterruptControl};
use esp_hal::peripherals::{CPU_CTRL, SW_INTERRUPT, SYSTIMER};
use esp_hal::system::Stack;
use esp_hal::timer::systimer::SystemTimer;
use esp_rtos::embassy::Executor;
use static_cell::StaticCell;

const APP_CORE_STACK: usize = 8_192;

/// The scheduler's own peripherals, so an application never has to name esp-rtos.
/// Built by [`crate::split!`].
pub struct Rtos {
    #[doc(hidden)]
    pub systimer: SYSTIMER<'static>,
    #[doc(hidden)]
    pub software_interrupt: SW_INTERRUPT<'static>,
    #[doc(hidden)]
    pub cpu_control: CPU_CTRL<'static>,
}

/// esp-rtos requires this on the first core.
fn start_scheduler(rtos: Rtos) -> (SoftwareInterrupt<'static, 1>, CPU_CTRL<'static>) {
    // SYSTIMER for embassy time, so it never contends with the step ISR on TIMG0.
    let systimer = SystemTimer::new(rtos.systimer);
    let software_interrupt = SoftwareInterruptControl::new(rtos.software_interrupt);
    esp_rtos::start(systimer.alarm0, software_interrupt.software_interrupt0);
    (software_interrupt.software_interrupt1, rtos.cpu_control)
}

/// Runs the board's executor on the calling core and never returns. The step
/// ISR, the queue refill and the application all share that core.
pub fn boot(rtos: Rtos, init: impl FnOnce(Spawner)) -> ! {
    let _ = start_scheduler(rtos);
    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    EXECUTOR.init(Executor::new()).run(init)
}

/// Runs the board's executor on the second core and returns with the calling
/// core free for the application. The tick ISR is installed by whichever core
/// calls `stepper::start`, so building the `Board` inside `init` pins step
/// generation to the second core too.
pub fn boot_on_second_core(rtos: Rtos, init: impl FnOnce(Spawner) + Send + 'static) {
    let (software_interrupt, cpu_control) = start_scheduler(rtos);
    static STACK: StaticCell<Stack<APP_CORE_STACK>> = StaticCell::new();
    esp_rtos::start_second_core(
        cpu_control,
        software_interrupt,
        STACK.init(Stack::new()),
        move || {
            static EXECUTOR: StaticCell<Executor> = StaticCell::new();
            EXECUTOR.init(Executor::new()).run(init)
        },
    );
}
