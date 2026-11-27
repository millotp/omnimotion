//! Recent log lines in a fixed ring, so they can be served without a heap
//! buffer for every line.

/// The longest line kept; a longer one is cut at a character boundary.
pub const MAX_LINE: usize = 240;

/// The newest log text in `N` bytes. Each line is stored with a newline after
/// it; once full, the oldest bytes are overwritten.
pub struct LogBuffer<const N: usize> {
    bytes: [u8; N],
    next: usize,
    filled: usize,
}

impl<const N: usize> Default for LogBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> LogBuffer<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            next: 0,
            filled: 0,
        }
    }

    pub fn push_line(&mut self, line: &str) {
        let line = truncate(line, MAX_LINE);
        for &byte in line.as_bytes().iter().chain(core::iter::once(&b'\n')) {
            self.bytes[self.next] = byte;
            self.next = (self.next + 1) % N;
            self.filled = (self.filled + 1).min(N);
        }
    }

    /// The stored text, oldest first, copied into `out`. The oldest line can
    /// begin cut off where its first bytes were overwritten, but never inside
    /// a character.
    pub fn text<'a>(&self, out: &'a mut [u8; N]) -> &'a str {
        let start = (self.next + N - self.filled) % N;
        for (offset, byte) in out.iter_mut().take(self.filled).enumerate() {
            *byte = self.bytes[(start + offset) % N];
        }
        let text = &out[..self.filled];
        let first = text
            .iter()
            .position(|&byte| byte & 0xC0 != 0x80)
            .unwrap_or(text.len());
        core::str::from_utf8(&text[first..]).unwrap_or("")
    }
}

fn truncate(line: &str, max: usize) -> &str {
    if line.len() <= max {
        return line;
    }
    let mut end = max;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}
