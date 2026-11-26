use core::sync::atomic::{AtomicU32, Ordering};

use embassy_time::{Duration, Timer};
use esp_hal::gpio::{Input, Output};
use esp_hal::timer::timg::MwdtStage;
use omnimotion::devices::endstop::{EndstopBank, EndstopConfig, EndstopRole, ENDSTOPS};
use omnimotion::AXES;

use crate::motion::stepper;
use crate::motion::Motion;

const POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Protection flags only clear while ENN is inactive.
const RESET_PULSE: Duration = Duration::from_millis(10);

static LATCHED: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Fault {
    EStop = 1 << 0,
    Endstop = 1 << 1,
    Diag = 1 << 2,
    Underrun = 1 << 3,
    MotionEngine = 1 << 4,
    Driver = 1 << 5,
}

/// What a homing pass waits for; handed back unlatched instead of faulting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HomingTrigger {
    Diag(usize),
    Endstop(usize),
}

pub fn latched() -> u32 {
    LATCHED.load(Ordering::Acquire)
}

pub(crate) fn latch(fault: Fault) {
    LATCHED.fetch_or(fault as u32, Ordering::AcqRel);
}

pub(crate) fn clear_latch() {
    LATCHED.store(0, Ordering::Release);
}

pub struct FaultSupervisor {
    watchdog: stepper::Watchdog,
    #[cfg_attr(feature = "virtual-hardware", allow(dead_code))]
    estop: Input<'static>,
    drv_en: Output<'static>,
    diag: [Input<'static>; AXES],
    #[cfg_attr(feature = "virtual-hardware", allow(dead_code))]
    endstops: [Input<'static>; ENDSTOPS],
    bank: EndstopBank,
}

impl FaultSupervisor {
    pub(crate) fn new(
        mut watchdog: stepper::Watchdog,
        estop: Input<'static>,
        drv_en: Output<'static>,
        diag: [Input<'static>; AXES],
        endstops: [Input<'static>; ENDSTOPS],
        config: [EndstopConfig; ENDSTOPS],
    ) -> Self {
        watchdog.set_timeout(MwdtStage::Stage0, esp_hal::time::Duration::from_secs(5));
        Self {
            watchdog,
            estop,
            drv_en,
            diag,
            endstops,
            bank: EndstopBank::new(config),
        }
    }

    /// E-stop pole A idles low through the closed contact and rises when the
    /// button opens or the cable breaks.
    #[cfg(not(feature = "virtual-hardware"))]
    pub fn estop_closed(&self) -> bool {
        self.estop.is_low()
    }

    /// There is no button on a board running virtual hardware.
    #[cfg(feature = "virtual-hardware")]
    pub fn estop_closed(&self) -> bool {
        true
    }

    pub fn drivers_enabled(&self) -> bool {
        self.drv_en.is_set_high()
    }

    /// Arms the watchdog too: from here until `disable_drivers`, something must
    /// keep polling this supervisor or the MCU resets with ENN pulled inactive.
    pub fn enable_drivers(&mut self) {
        self.watchdog.enable();
        self.watchdog.feed();
        self.drv_en.set_high();
    }

    pub fn disable_drivers(&mut self) {
        self.drv_en.set_low();
        self.watchdog.disable();
    }

    /// Clears latched driver protection while the outputs stay disabled.
    pub async fn reset_driver_fault(&mut self) {
        self.disable_drivers();
        Timer::after(RESET_PULSE).await;
    }

    pub fn diag_asserted(&self, axis: usize) -> bool {
        self.diag[axis].is_high()
    }

    pub fn endstop_config(&self, endstop: usize) -> Option<EndstopConfig> {
        self.bank.config(endstop)
    }

    /// Whether a connector reads triggered right now, undebounced.
    pub fn endstop_asserted(&self, endstop: usize) -> bool {
        self.bank.triggered_now(self.port()) & (1 << endstop) != 0
    }

    /// The six inputs as one bitmask, in the order `EndstopBank` expects. Each
    /// idles high through its pull-up.
    #[cfg(not(feature = "virtual-hardware"))]
    pub(crate) fn port(&self) -> u8 {
        let mut port = 0;
        for (endstop, pin) in self.endstops.iter().enumerate() {
            if pin.is_high() {
                port |= 1 << endstop;
            }
        }
        port
    }

    /// The levels the installed machine model implies: each input is low while its endstop is
    /// triggered, or high while it is, for a connector wired the other way round.
    #[cfg(feature = "virtual-hardware")]
    pub(crate) fn port(&self) -> u8 {
        let triggered = crate::virtual_hardware::triggered();
        let mut port = 0;
        for endstop in 0..ENDSTOPS {
            let active_low = self
                .bank
                .config(endstop)
                .map_or(true, |config| config.active_low);
            if (triggered & (1 << endstop) != 0) != active_low {
                port |= 1 << endstop;
            }
        }
        port
    }

    pub async fn trip(&mut self, fault: Fault) {
        self.watchdog.feed();
        latch(fault);
        let generation = stepper::abort();
        self.disable_drivers();
        stepper::await_abort(generation).await;
    }

    /// Clears faults only while disabled and healthy.
    pub fn clear(&mut self, motion: &mut Motion) -> bool {
        self.watchdog.feed();
        // Refusing on a reference would strand a machine parked on its datum.
        let limits = self.bank.role_mask(EndstopRole::Limit);
        if self.drivers_enabled()
            || !self.estop_closed()
            || self.diag.iter().any(Input::is_high)
            || self.bank.triggered_now(self.port()) & limits != 0
            || !motion.clear_fault()
        {
            return false;
        }
        clear_latch();
        true
    }

    /// Limits debounce against their configured type; DIAG and e-stop trip
    /// immediately. References are ignored: a job is expected to drive into them.
    pub async fn wait_for_fault(&mut self) -> Fault {
        self.wait_for_event(None, None)
            .await
            .expect("ordinary supervision never treats DIAG as a homing event")
    }

    /// Returns `None` when the homing trigger fires; other faults are latched.
    pub(crate) async fn wait_for_homing_event(&mut self, trigger: HomingTrigger) -> Option<Fault> {
        self.wait_for_event(Some(trigger), None).await
    }

    /// Supervises the retreat off the datum. A StallGuard pass must ignore its
    /// own DIAG, which stays asserted until the load comes off.
    pub(crate) async fn wait_for_backoff_fault(&mut self, trigger: HomingTrigger) -> Fault {
        let ignored_diag = match trigger {
            HomingTrigger::Diag(axis) => Some(axis),
            HomingTrigger::Endstop(_) => None,
        };
        self.wait_for_event(None, ignored_diag)
            .await
            .expect("backoff ignores its selected DIAG instead of returning it")
    }

    async fn wait_for_event(
        &mut self,
        trigger: Option<HomingTrigger>,
        ignored_diag: Option<usize>,
    ) -> Option<Fault> {
        self.watchdog.feed();
        self.bank.reset();

        loop {
            self.watchdog.feed();
            if stepper::underrun() {
                self.trip(Fault::Underrun).await;
                return Some(Fault::Underrun);
            }
            if stepper::engine_fault() {
                self.trip(Fault::MotionEngine).await;
                return Some(Fault::MotionEngine);
            }
            if !self.estop_closed() {
                self.trip(Fault::EStop).await;
                return Some(Fault::EStop);
            }

            for axis in 0..AXES {
                if self.diag[axis].is_high() {
                    if trigger == Some(HomingTrigger::Diag(axis)) {
                        return None;
                    }
                    if ignored_diag == Some(axis) {
                        continue;
                    }
                    self.trip(Fault::Diag).await;
                    return Some(Fault::Diag);
                }
            }

            let triggered = self.bank.sample(self.port());
            if let Some(HomingTrigger::Endstop(endstop)) = trigger {
                if triggered & (1 << endstop) != 0 {
                    return None;
                }
            }
            // References are deliberately driven into, so only limits trip.
            if triggered & self.bank.role_mask(EndstopRole::Limit) != 0 {
                self.trip(Fault::Endstop).await;
                return Some(Fault::Endstop);
            }

            Timer::after(POLL_INTERVAL).await;
        }
    }
}
