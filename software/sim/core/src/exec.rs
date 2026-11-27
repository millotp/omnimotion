//! The ESP32-S3's two cores, each running one embassy executor. They take turns on the host thread; since
//! tasks take no virtual time between awaits, the interleaving does not change what the firmware sees.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use embassy_executor::{raw, Spawner};

#[derive(Clone, Copy)]
struct Core {
    pending: &'static AtomicBool,
    executor: &'static raw::Executor,
}

// Only ever touched from the one host thread.
unsafe impl Send for Core {}

static CORES: Mutex<Vec<Core>> = Mutex::new(Vec::new());
/// Set with any core's `pending` flag, so that `poll_all` can skip `CORES` when nothing is pending.
static ANY_PENDING: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// `poll_all`'s copy of `CORES`, reused so that a pass does not allocate.
    static SNAPSHOT: Cell<Vec<Core>> = const { Cell::new(Vec::new()) };
}

/// A new executor, polled by the run loop from now on.
pub fn new_core() -> Spawner {
    let pending: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(true)));
    let executor: &'static raw::Executor = Box::leak(Box::new(raw::Executor::new(
        pending as *const AtomicBool as *mut (),
    )));
    CORES.lock().unwrap().push(Core { pending, executor });
    ANY_PENDING.store(true, Ordering::SeqCst);
    executor.spawner()
}

#[no_mangle]
fn __pender(context: *mut ()) {
    // Safety: every context is the `pending` flag leaked in `new_core`.
    unsafe { &*(context as *const AtomicBool) }.store(true, Ordering::SeqCst);
    ANY_PENDING.store(true, Ordering::SeqCst);
}

/// Polls every executor until none has work left at the current instant.
pub(crate) fn poll_all() {
    let mut cores = SNAPSHOT.take();
    while ANY_PENDING.swap(false, Ordering::SeqCst) {
        cores.clear();
        cores.extend_from_slice(&CORES.lock().unwrap());
        for core in &cores {
            if core.pending.swap(false, Ordering::SeqCst) {
                // Safety: executors are leaked, so 'static, and polled from this thread only.
                unsafe { core.executor.poll() };
            }
        }
    }
    SNAPSHOT.set(cores);
}
