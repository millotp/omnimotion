//! Virtual time. Nothing here reads a wall clock: time only moves when the run loop jumps it to the next
//! event, so the host can run it slower or faster than real time, or as fast as the CPU allows.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::task::Waker;

/// Microseconds since the simulated power-on: embassy-time's default 1 MHz tick.
static NOW: AtomicU64 = AtomicU64::new(0);
static WAKES: Mutex<Vec<(u64, Waker)>> = Mutex::new(Vec::new());
/// The earliest time in `WAKES` and how many entries it has, readable without the lock.
static EARLIEST: AtomicU64 = AtomicU64::new(u64::MAX);
static QUEUED: AtomicUsize = AtomicUsize::new(0);
/// `fire_due`'s list of wakers to wake, kept so that each wake-up does not allocate.
static DUE: Mutex<Vec<Waker>> = Mutex::new(Vec::new());

pub fn now() -> u64 {
    NOW.load(Ordering::Relaxed)
}

pub(crate) fn set(t: u64) {
    NOW.store(t, Ordering::Relaxed);
}

/// Wakes `waker` once virtual time reaches `at`.
pub fn wake_at(at: u64, waker: &Waker) {
    let mut wakes = WAKES.lock().unwrap();
    if let Some(entry) = wakes.iter_mut().find(|(_, w)| w.will_wake(waker)) {
        entry.0 = entry.0.min(at);
    } else {
        wakes.push((at, waker.clone()));
    }
    EARLIEST.fetch_min(at, Ordering::SeqCst);
    QUEUED.store(wakes.len(), Ordering::SeqCst);
}

pub(crate) fn next_wake() -> Option<u64> {
    (QUEUED.load(Ordering::SeqCst) > 0).then(|| EARLIEST.load(Ordering::SeqCst))
}

pub(crate) fn fire_due(now: u64) {
    if QUEUED.load(Ordering::SeqCst) == 0 || EARLIEST.load(Ordering::SeqCst) > now {
        return;
    }
    let mut due = std::mem::take(&mut *DUE.lock().unwrap());
    {
        let mut wakes = WAKES.lock().unwrap();
        wakes.retain(|(at, waker)| {
            if *at <= now {
                due.push(waker.clone());
                false
            } else {
                true
            }
        });
        EARLIEST.store(
            wakes.iter().map(|(at, _)| *at).min().unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
        QUEUED.store(wakes.len(), Ordering::SeqCst);
    }
    for waker in due.drain(..) {
        waker.wake();
    }
    *DUE.lock().unwrap() = due;
}

struct SimDriver;

impl embassy_time_driver::Driver for SimDriver {
    fn now(&self) -> u64 {
        now()
    }

    fn schedule_wake(&self, at: u64, waker: &Waker) {
        wake_at(at, waker);
    }
}

embassy_time_driver::time_driver_impl!(static DRIVER: SimDriver = SimDriver);
