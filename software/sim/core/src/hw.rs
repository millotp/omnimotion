//! The ESP32-S3 peripherals the board code drives directly: the GPIO output register, TIMG0's timer 0
//! alarm that runs the step ISR, and TIMG0's watchdog.

pub const STEP: [u8; 3] = [4, 5, 6];
pub const DIR: [u8; 3] = [7, 8, 9];
pub const DIAG: [u8; 3] = [10, 11, 12];
pub const ENDSTOPS: [u8; 6] = [38, 39, 40, 41, 13, 14];
pub const ESTOP: u8 = 15;
pub const DRV_EN: u8 = 21;
const PINS: usize = 49;
/// ISR entries at one instant before the simulator gives up: a handler that never clears its interrupt,
/// or an alarm the counter has already passed, would hold the CPU for good on the chip.
const ISR_STORM: u32 = 1_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Pull {
    #[default]
    None,
    Up,
    Down,
}

/// TIMG0 timer 0, counting up one tick per µs.
#[derive(Default)]
pub struct Timer0 {
    pub en: bool,
    pub alarm_en: bool,
    pub int_en: bool,
    pub auto_reload: bool,
    /// Alarm value, ticks.
    pub alarm: u64,
    /// LOADLO/LOADHI: what a write to LOAD, or the alarm with auto-reload, puts in the counter.
    pub reload: u64,
    /// Raw interrupt status: set by the alarm, cleared only by INT_CLR. The interrupt is level-triggered.
    pub int_raw: bool,
    /// While counting: the time at which the counter would have read zero.
    base: i64,
    /// While stopped: the counter value.
    held: u64,
}

impl Timer0 {
    pub fn counter(&self, now: u64) -> u64 {
        if self.en {
            (now as i64 - self.base).max(0) as u64
        } else {
            self.held
        }
    }

    pub fn set_counter(&mut self, value: u64, now: u64) {
        if self.en {
            self.base = now as i64 - value as i64;
        } else {
            self.held = value;
        }
    }

    /// Starts or stops the counter; it resumes from where it stopped.
    pub fn set_enabled(&mut self, en: bool, now: u64) {
        let value = self.counter(now);
        self.en = en;
        self.set_counter(value, now);
    }

    /// When the counter reaches the alarm value; the alarm also fires at once on a counter already past it.
    fn alarm_at(&self) -> Option<u64> {
        (self.en && self.alarm_en).then(|| (self.base + self.alarm as i64).max(0) as u64)
    }
}

#[derive(Default)]
pub struct Wdt {
    pub timeout: u64,
    pub deadline: Option<u64>,
}

pub struct Hw {
    pub out: u64,
    pub pull: [Pull; PINS],
    pub timer: Timer0,
    pub isr: Option<fn()>,
    pub wdt: Wdt,
    /// When the ISR last ran, and how many times in a row at that instant.
    last_isr: (u64, u32),
}

impl Hw {
    /// When the step ISR runs next: at once while its interrupt is pending, else at the alarm.
    pub fn next_isr(&self) -> Option<u64> {
        let t = &self.timer;
        if !t.int_en || self.isr.is_none() {
            return None;
        }
        if t.int_raw {
            return Some(0);
        }
        t.alarm_at()
    }

    /// Virtual time has reached `now`: fires the alarm if it is due (which clears ALARM_EN and, with
    /// auto-reload, reloads the counter) and returns the handler to run if the interrupt is pending.
    pub fn alarm(&mut self, now: u64) -> Option<fn()> {
        let t = &mut self.timer;
        let fired = t.alarm_at().is_some_and(|at| at <= now);
        if fired {
            t.alarm_en = false;
            t.int_raw = true;
            if t.auto_reload {
                let reload = t.reload;
                t.set_counter(reload, now);
            }
        }
        if !(t.int_raw && t.int_en) {
            return None;
        }
        let repeats = if self.last_isr.0 == now {
            self.last_isr.1 + 1
        } else {
            0
        };
        self.last_isr = (now, repeats);
        if repeats >= ISR_STORM {
            if fired {
                panic!("the TIMG0 alarm keeps firing at one instant: the counter is past the alarm (no auto-reload?)");
            }
            panic!("the TIMG0 interrupt is never cleared (INT_CLR): its handler would run forever");
        }
        self.isr
    }

    pub fn level(&self, pin: u8) -> bool {
        self.out & (1 << pin) != 0
    }
}

impl Default for Hw {
    fn default() -> Self {
        Self {
            out: 0,
            pull: [Pull::None; PINS],
            timer: Timer0::default(),
            isr: None,
            wdt: Wdt::default(),
            last_isr: (u64::MAX, 0),
        }
    }
}
