use core::cell::Cell;
#[cfg(not(feature = "virtual-hardware"))]
use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_time::{Duration, Timer};
use esp_hal::gpio::{Input, Output};
use esp_hal::timer::timg::MwdtStage;
use omnimotion::devices::endstop::{self, EndstopBank, EndstopConfig, EndstopRole, ENDSTOPS};
use omnimotion::devices::power::{PowerSource, Rails};
use omnimotion::AXES;

use crate::diag::inventory::Vmon;
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
    /// VMOTOR left the motor-ready window with the outputs on. The TMC2209s
    /// run their logic off it, so they come back with power-on defaults
    /// (strapped microsteps, full current) and must be configured again before
    /// another step; `Board::enable` does that.
    Supply = 1 << 6,
}

impl Fault {
    const ALL: [Self; 7] = [
        Self::EStop,
        Self::Endstop,
        Self::Diag,
        Self::Underrun,
        Self::MotionEngine,
        Self::Driver,
        Self::Supply,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::EStop => "emergency stop",
            Self::Endstop => "limit switch",
            Self::Diag => "driver diagnostic",
            Self::Underrun => "step underrun",
            Self::MotionEngine => "motion engine",
            Self::Driver => "motor driver",
            Self::Supply => "motor supply lost",
        }
    }

    /// Whether `bits`, as `latched` returns them, include this fault.
    pub fn is_in(self, bits: u32) -> bool {
        bits & self as u32 != 0
    }

    /// The names of the latched `bits`: one fault's name, or "multiple faults".
    pub fn describe(bits: u32) -> &'static str {
        Self::ALL
            .into_iter()
            .find(|fault| *fault as u32 == bits)
            .map_or("multiple faults", Self::name)
    }
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

/// The endstop inputs and their wiring live outside the supervisor, so they
/// can be read from anywhere, even while a move has the supervisor watching.
#[cfg(not(feature = "virtual-hardware"))]
static ENDSTOP_PINS: Mutex<CriticalSectionRawMutex, RefCell<Option<[Input<'static>; ENDSTOPS]>>> =
    Mutex::new(RefCell::new(None));
static ENDSTOP_CONFIG: Mutex<CriticalSectionRawMutex, Cell<[EndstopConfig; ENDSTOPS]>> =
    Mutex::new(Cell::new([EndstopConfig::UNUSED; ENDSTOPS]));

/// The six inputs as one bitmask, bit n the level of En. Each idles high
/// through its pull-up.
#[cfg(not(feature = "virtual-hardware"))]
pub(crate) fn endstop_levels() -> u8 {
    ENDSTOP_PINS.lock(|pins| {
        pins.borrow().as_ref().map_or(u8::MAX, |pins| {
            pins.iter()
                .enumerate()
                .filter(|(_, pin)| pin.is_high())
                .fold(0, |levels, (endstop, _)| levels | 1 << endstop)
        })
    })
}

/// The levels the installed machine model implies: each input is low while its endstop is
/// triggered, or high while it is, for a connector wired the other way round.
#[cfg(feature = "virtual-hardware")]
pub(crate) fn endstop_levels() -> u8 {
    let triggered = crate::virtual_hardware::triggered();
    let config = ENDSTOP_CONFIG.lock(Cell::get);
    (0..ENDSTOPS)
        .filter(|&endstop| (triggered & (1 << endstop) != 0) != config[endstop].active_low)
        .fold(0, |levels, endstop| levels | 1 << endstop)
}

/// The fitted endstops triggered right now (bit n is En), undebounced. Cheap
/// enough to sample every few hundred microseconds during a move.
pub fn endstops_triggered() -> u8 {
    endstop::triggered(&ENDSTOP_CONFIG.lock(Cell::get), endstop_levels())
}

pub struct FaultSupervisor {
    watchdog: stepper::Watchdog,
    #[cfg_attr(feature = "virtual-hardware", allow(dead_code))]
    estop: Input<'static>,
    drv_en: Output<'static>,
    diag: [Input<'static>; AXES],
    bank: EndstopBank,
    vmon: Vmon,
}

impl FaultSupervisor {
    pub(crate) fn new(
        mut watchdog: stepper::Watchdog,
        estop: Input<'static>,
        drv_en: Output<'static>,
        diag: [Input<'static>; AXES],
        endstops: [Input<'static>; ENDSTOPS],
        config: [EndstopConfig; ENDSTOPS],
        vmon: Vmon,
    ) -> Self {
        watchdog.set_timeout(MwdtStage::Stage0, esp_hal::time::Duration::from_secs(5));
        #[cfg(not(feature = "virtual-hardware"))]
        ENDSTOP_PINS.lock(|pins| *pins.borrow_mut() = Some(endstops));
        #[cfg(feature = "virtual-hardware")]
        let _ = endstops;
        ENDSTOP_CONFIG.lock(|stored| stored.set(config));
        Self {
            watchdog,
            estop,
            drv_en,
            diag,
            bank: EndstopBank::new(config),
            vmon,
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

    /// The rails as VMOTOR reads now.
    pub fn rails(&mut self) -> Rails {
        self.vmon.read()
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
        endstops_triggered() & (1 << endstop) != 0
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
            || endstops_triggered() & limits != 0
            || !motion.clear_fault()
        {
            return false;
        }
        clear_latch();
        true
    }

    /// Limits debounce against their configured type; DIAG, the e-stop and
    /// the motor supply trip immediately. References are ignored: a job is
    /// expected to drive into them.
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
            if self.drivers_enabled() && self.vmon.read().source() != PowerSource::Bulk {
                self.trip(Fault::Supply).await;
                return Some(Fault::Supply);
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

            let triggered = self.bank.sample(endstop_levels());
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
