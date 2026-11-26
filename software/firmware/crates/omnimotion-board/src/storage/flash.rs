//! The flash chip is shared by NVS on the board's core and OTA on the network
//! core. Every erase or write masks interrupts on the calling core and parks the
//! other, so each sector stalls the whole chip for tens of milliseconds.
//!
//! Two rules follow. Only one flash operation may run at a time, or one core
//! could park the other in the middle of its own. And an OTA write must never
//! land while a move is feeding the step ISR, or the queue underruns.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use embassy_time::Timer;

const FREE: u8 = 0;
const BRIEF: u8 = 1;
const UPLOAD: u8 = 2;

static IN_USE: AtomicBool = AtomicBool::new(false);
static MOVING: AtomicBool = AtomicBool::new(false);
static LEASE: AtomicU8 = AtomicU8::new(FREE);

/// Runs one flash operation with the chip to itself. `operation` must not
/// await: the other core spins until it returns.
pub(crate) fn exclusive<R>(operation: impl FnOnce() -> R) -> R {
    while IN_USE
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let result = operation();
    IN_USE.store(false, Ordering::Release);
    result
}

// `Lease` and `Moving` each publish their own flag before reading the other's,
// all SeqCst, so at most one of two racing claims can succeed.

/// Keeps moves from starting while OTA writes to flash.
pub(crate) struct Lease(());

impl Lease {
    /// For a single otadata update. A move that starts meanwhile waits it out.
    pub(crate) fn try_brief() -> Option<Self> {
        Self::try_acquire(BRIEF)
    }

    /// For a whole image. Moves fail with `Busy` until it is dropped.
    pub(crate) fn try_upload() -> Option<Self> {
        Self::try_acquire(UPLOAD)
    }

    /// `None` while a move is running or another lease is held.
    fn try_acquire(kind: u8) -> Option<Self> {
        if LEASE
            .compare_exchange(FREE, kind, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return None;
        }
        if MOVING.load(Ordering::SeqCst) {
            LEASE.store(FREE, Ordering::SeqCst);
            return None;
        }
        Some(Self(()))
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        LEASE.store(FREE, Ordering::SeqCst);
    }
}

/// Held by `Motion` for the whole of a move.
pub(crate) struct Moving(());

impl Moving {
    /// `None` while an upload holds the lease.
    pub(crate) async fn begin() -> Option<Self> {
        loop {
            MOVING.store(true, Ordering::SeqCst);
            match LEASE.load(Ordering::SeqCst) {
                FREE => return Some(Self(())),
                UPLOAD => {
                    MOVING.store(false, Ordering::SeqCst);
                    return None;
                }
                _ => {
                    MOVING.store(false, Ordering::SeqCst);
                    Timer::after_millis(1).await;
                }
            }
        }
    }
}

impl Drop for Moving {
    fn drop(&mut self) {
        MOVING.store(false, Ordering::SeqCst);
    }
}
