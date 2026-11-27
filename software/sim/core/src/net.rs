//! The WiFi side, as seen from the firmware: TCP connections to port 80. The host opens one per HTTP
//! request with the request bytes and collects the response once the firmware closes it.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::task::Waker;

#[derive(Default)]
struct Conn {
    request: VecDeque<u8>,
    response: Vec<u8>,
    closed: bool,
    waker: Option<Waker>,
}

#[derive(Default)]
struct Net {
    next: u32,
    backlog: VecDeque<u32>,
    conns: HashMap<u32, Conn>,
    listeners: Vec<Waker>,
}

static NET: Mutex<Option<Net>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Net) -> R) -> R {
    let mut net = NET.lock().unwrap();
    f(net.get_or_insert_with(Net::default))
}

/// Host side: a client connects and sends `request`. Returns the connection id.
pub fn open(request: &[u8]) -> u32 {
    let (id, wake) = with(|net| {
        net.next += 1;
        let id = net.next;
        net.conns.insert(
            id,
            Conn {
                request: request.iter().copied().collect(),
                ..Conn::default()
            },
        );
        net.backlog.push_back(id);
        (id, std::mem::take(&mut net.listeners))
    });
    for w in wake {
        w.wake();
    }
    id
}

/// Host side: the whole response once the firmware has closed the connection.
pub fn response(id: u32) -> Option<Vec<u8>> {
    with(|net| {
        let done = net.conns.get(&id).is_some_and(|c| c.closed);
        done.then(|| {
            net.conns
                .remove(&id)
                .map(|c| c.response)
                .unwrap_or_default()
        })
    })
}

/// Firmware side: the next connection waiting on port 80, or registers to be woken.
pub fn accept(waker: &Waker) -> Option<u32> {
    with(|net| {
        let id = net.backlog.pop_front();
        if id.is_none() {
            net.listeners.push(waker.clone());
        }
        id
    })
}

/// Firmware side: request bytes; `Some(0)` once the client has nothing more to send.
pub fn read(id: u32, buf: &mut [u8], waker: &Waker) -> Option<usize> {
    with(|net| {
        let conn = net.conns.get_mut(&id)?;
        if conn.request.is_empty() {
            conn.waker = Some(waker.clone());
            return None;
        }
        let n = buf.len().min(conn.request.len());
        for (slot, byte) in buf.iter_mut().zip(conn.request.drain(..n)) {
            *slot = byte;
        }
        Some(n)
    })
}

pub fn write(id: u32, data: &[u8]) {
    with(|net| {
        if let Some(conn) = net.conns.get_mut(&id) {
            conn.response.extend_from_slice(data);
        }
    });
}

pub fn close(id: u32) {
    with(|net| {
        if let Some(conn) = net.conns.get_mut(&id) {
            conn.closed = true;
        }
    });
}
