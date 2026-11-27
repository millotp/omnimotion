//! The board's recent log lines, for an application to serve.

use core::cell::RefCell;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use omnimotion::machine::log::LogBuffer;

/// How much recent text is kept.
pub const LOG_BYTES: usize = 2_048;

static LOG: Mutex<CriticalSectionRawMutex, RefCell<LogBuffer<LOG_BYTES>>> =
    Mutex::new(RefCell::new(LogBuffer::new()));

/// Keeps `line`; the oldest text goes once `LOG_BYTES` are held.
pub fn push(line: &str) {
    LOG.lock(|log| log.borrow_mut().push_line(line));
}

/// The kept text, oldest first, copied into `out`.
pub fn text(out: &mut [u8; LOG_BYTES]) -> &str {
    LOG.lock(|log| log.borrow().text(out))
}
