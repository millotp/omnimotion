//! The firmware's serial console and the simulator's own notes, timestamped in virtual time. Hosts drain it.

use std::sync::Mutex;

static LINES: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());
const KEEP: usize = 5_000;

pub fn line(t: u64, text: &str) {
    let mut lines = LINES.lock().unwrap();
    if lines.len() >= KEEP {
        lines.drain(..KEEP / 2);
    }
    lines.push((t, text.trim_end().to_string()));
}

pub fn take() -> Vec<(u64, String)> {
    std::mem::take(&mut *LINES.lock().unwrap())
}
