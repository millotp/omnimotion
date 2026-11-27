use crate::time::Duration;

#[derive(Debug)]
pub struct Error;

pub trait Timer {
    fn load_value(&self, value: Duration) -> Result<(), Error>;
    fn enable_auto_reload(&self, enable: bool);
    fn enable_interrupt(&self, enable: bool);
    fn start(&self);
    fn set_interrupt_handler(&self, handler: fn());
}

pub mod timg {
    use core::marker::PhantomData;

    use crate::peripherals::TIMG0;
    use crate::time::Duration;

    pub struct TimerGroup<'d> {
        pub timer0: Timer0<'d>,
        pub wdt: Wdt<TIMG0<'d>>,
    }

    impl<'d> TimerGroup<'d> {
        pub fn new(_timg: TIMG0<'d>) -> Self {
            Self {
                timer0: Timer0(PhantomData),
                wdt: Wdt(PhantomData),
            }
        }
    }

    pub struct Timer0<'d>(PhantomData<&'d mut ()>);

    impl super::Timer for Timer0<'_> {
        /// Sets the alarm value.
        fn load_value(&self, value: Duration) -> Result<(), super::Error> {
            omnimotion_sim::hal::timer(|t, _| t.alarm = value.as_micros());
            Ok(())
        }
        fn enable_auto_reload(&self, enable: bool) {
            omnimotion_sim::hal::timer(|t, _| t.auto_reload = enable);
        }
        fn enable_interrupt(&self, enable: bool) {
            omnimotion_sim::hal::timer(|t, _| t.int_en = enable);
        }
        /// As esp-hal: stop, load zero into the counter, then count with the alarm armed.
        fn start(&self) {
            omnimotion_sim::hal::timer(|t, now| {
                t.set_enabled(false, now);
                t.alarm_en = false;
                t.reload = 0;
                t.set_counter(0, now);
                t.set_enabled(true, now);
                t.alarm_en = true;
            });
        }
        fn set_interrupt_handler(&self, handler: fn()) {
            omnimotion_sim::hal::set_isr(handler);
        }
    }

    #[derive(Clone, Copy, Debug)]
    pub enum MwdtStage {
        Stage0,
    }

    pub struct Wdt<T>(PhantomData<T>);

    impl<T> Wdt<T> {
        pub fn set_timeout(&mut self, _stage: MwdtStage, timeout: Duration) {
            omnimotion_sim::hal::watchdog(|w, _| w.timeout = timeout.as_micros());
        }
        pub fn enable(&mut self) {
            omnimotion_sim::hal::watchdog(|w, now| w.deadline = Some(now + w.timeout));
        }
        pub fn feed(&mut self) {
            omnimotion_sim::hal::watchdog(|w, now| {
                if w.deadline.is_some() {
                    w.deadline = Some(now + w.timeout);
                }
            });
        }
        pub fn disable(&mut self) {
            omnimotion_sim::hal::watchdog(|w, _| w.deadline = None);
        }
    }
}
