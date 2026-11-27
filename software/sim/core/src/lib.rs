//! The Omnimotion board in simulation, with the machine an application builds on it.
//!
//! The host (a browser worker or a command-line tool) installs the world with its [`Plant`], calls the
//! firmware's `main`, and then repeatedly asks [`run_until`] to advance virtual time. Everything the firmware
//! can observe goes through the stand-in crates, which call [`hal`].

pub mod clock;
pub mod exec;
pub mod hal;
pub mod hw;
pub mod log;
pub mod net;
pub mod stepper;
pub mod tmc;

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde::{Deserialize, Serialize};

/// The machine on the board: what the motors drive and what the endstop inputs see.
pub trait Plant: Any + Send {
    /// When the plant next needs [`Plant::step`], while anything moves or settles.
    fn next_step(&self) -> Option<u64>;
    /// Virtual time has reached `now`: integrate against the drivers' fields and currents.
    fn step(&mut self, drivers: &tmc::Drivers, now: u64);
    /// A step pulse or a change of the drivers' enable at `now`.
    fn wake(&mut self, now: u64);
    /// The level the machine drives on endstop connector `endstop` (true high), or `None` where nothing is
    /// connected and the input reads its pull.
    fn endstop_level(&self, endstop: usize, now: u64) -> Option<bool>;
    /// What survives an MCU reset, for the host's restore function.
    fn save(&self) -> serde_json::Value;
}

/// The board as fitted.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Board {
    /// VMOTOR while the supply is on, V.
    pub supply_v: f64,
    /// Which of the three driver sockets (M0 to M2) hold a TMC2209.
    pub fitted: [bool; 3],
}

/// What the operator controls outside the firmware.
#[derive(Clone, Serialize, Deserialize)]
pub struct Env {
    pub supply_on: bool,
    pub estop_closed: bool,
}

pub struct World {
    pub board: Board,
    pub hw: hw::Hw,
    pub drivers: tmc::Drivers,
    pub plant: Box<dyn Plant>,
    pub env: Env,
    /// The `nvs` partition: blobs keyed by "namespace/key".
    pub nvs: BTreeMap<String, Vec<u8>>,
    pub boots: u32,
}

impl World {
    /// The installed plant, as the type the application gave.
    pub fn plant<P: Plant>(&self) -> &P {
        (&*self.plant as &dyn Any)
            .downcast_ref()
            .expect("the installed plant type")
    }

    pub fn plant_mut<P: Plant>(&mut self) -> &mut P {
        (&mut *self.plant as &mut dyn Any)
            .downcast_mut()
            .expect("the installed plant type")
    }
}

/// Everything that survives an MCU reset: the machine, the drivers' power and registers, and flash.
#[derive(Serialize, Deserialize)]
struct Saved {
    now: u64,
    board: Board,
    env: Env,
    drivers: tmc::Drivers,
    plant: serde_json::Value,
    nvs: BTreeMap<String, Vec<u8>>,
    boots: u32,
}

/// What only the host can do.
pub struct Host {
    /// Called once the firmware's last executor is running: returns control to the host for good.
    pub park: fn() -> !,
    /// Restarts the firmware from scratch with this saved world state.
    pub restart: fn(String) -> !,
}

static HOST: OnceLock<Host> = OnceLock::new();
static WORLD: Mutex<Option<World>> = Mutex::new(None);

fn lock() -> MutexGuard<'static, Option<World>> {
    WORLD.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn with<R>(f: impl FnOnce(&mut World) -> R) -> R {
    f(lock().as_mut().expect("world installed"))
}

/// Installs the world: fresh, with `board` and the plant `plant(None)` builds, or from a state saved by a
/// reset, with the plant `plant(Some(saved))` restores.
pub fn install(
    host: Host,
    board: Board,
    saved: Option<&str>,
    plant: fn(Option<serde_json::Value>) -> Box<dyn Plant>,
) {
    let _ = HOST.set(host);
    let world = match saved.and_then(|s| serde_json::from_str::<Saved>(s).ok()) {
        Some(s) => {
            clock::set(s.now);
            World {
                board: s.board,
                hw: hw::Hw::default(),
                drivers: s.drivers,
                plant: plant(Some(s.plant)),
                env: s.env,
                nvs: s.nvs,
                boots: s.boots + 1,
            }
        }
        None => {
            let mut drivers = tmc::Drivers::new(board.fitted);
            drivers.set_power(true, 0);
            World {
                board,
                hw: hw::Hw::default(),
                drivers,
                plant: plant(None),
                env: Env {
                    supply_on: true,
                    estop_closed: true,
                },
                nvs: BTreeMap::new(),
                boots: 1,
            }
        }
    };
    *lock() = Some(world);
}

pub fn park() -> ! {
    (HOST.get().expect("host installed").park)()
}

/// The MCU resets: everything outside it carries on.
pub fn reset(reason: &str) -> ! {
    let state = with(|w| {
        w.drivers.enabled = false;
        log::line(clock::now(), &format!("[sim] MCU reset: {reason}"));
        serde_json::to_string(&Saved {
            now: clock::now(),
            board: w.board,
            env: w.env.clone(),
            drivers: std::mem::take(&mut w.drivers),
            plant: w.plant.save(),
            nvs: w.nvs.clone(),
            boots: w.boots,
        })
        .expect("serialisable")
    });
    (HOST.get().expect("host installed").restart)(state)
}

/// Advances virtual time event by event up to `target` (µs). `budget` is asked every few thousand events
/// whether the host wants control back. Returns the time reached.
pub fn run_until(target: u64, budget: &mut dyn FnMut() -> bool) -> u64 {
    let mut events = 0u32;
    loop {
        exec::poll_all();
        let now = clock::now();
        let (isr, step, watchdog) =
            with(|w| (w.hw.next_isr(), w.plant.next_step(), w.hw.wdt.deadline));
        let next = [clock::next_wake(), isr, step, watchdog]
            .into_iter()
            .flatten()
            .min();
        let t = match next {
            Some(t) if t <= target => t.max(now),
            _ => {
                if target != u64::MAX && target > now {
                    clock::set(target);
                }
                return clock::now();
            }
        };
        clock::set(t);
        if watchdog.is_some_and(|d| d <= t) {
            reset("TIMG0 watchdog expired");
        }
        if isr.is_some_and(|i| i <= t) {
            if let Some(handler) = with(|w| w.hw.alarm(t)) {
                handler();
            }
        }
        if step.is_some_and(|s| s <= t) {
            with(|w| w.plant.step(&w.drivers, t));
        }
        clock::fire_due(t);
        events += 1;
        if events.is_multiple_of(4096) && budget() {
            return t;
        }
    }
}
