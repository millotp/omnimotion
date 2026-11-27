//! Moving HTTP bytes over a socket; `omnimotion::net::http` parses them.

use core::fmt::Write as _;

use embassy_net::tcp::TcpSocket;
use embedded_io_async::Write;
use heapless::String;
use omnimotion::net::http::{head_end, reason_phrase, RequestError};

/// Reads until a whole head is in `buffer`, and returns its length (through
/// the blank line) and how much of `buffer` was filled: the bytes between the
/// two are the start of the body.
pub async fn read_head(
    socket: &mut TcpSocket<'_>,
    buffer: &mut [u8],
) -> Result<(usize, usize), RequestError> {
    let mut filled = 0;
    loop {
        if let Some(end) = head_end(&buffer[..filled]) {
            return Ok((end, filled));
        }
        if filled == buffer.len() {
            return Err(RequestError::HeadTooLarge);
        }
        let read = socket
            .read(&mut buffer[filled..])
            .await
            .map_err(|_| RequestError::Io)?;
        if read == 0 {
            return Err(RequestError::Malformed);
        }
        filled += read;
    }
}

/// Hands a body of `length` bytes to `sink` as it arrives, starting with
/// `early`, what `read_head` read past the head. A connection that ends or
/// fails first gives `truncated`.
pub async fn read_body<E>(
    socket: &mut TcpSocket<'_>,
    early: &[u8],
    length: usize,
    truncated: E,
    mut sink: impl FnMut(&[u8]) -> Result<(), E>,
) -> Result<(), E> {
    let early = &early[..early.len().min(length)];
    if !early.is_empty() {
        sink(early)?;
    }
    let mut buffer = [0; 2_048];
    let mut remaining = length - early.len();
    while remaining > 0 {
        let wanted = remaining.min(buffer.len());
        match socket.read(&mut buffer[..wanted]).await {
            Ok(read) if read > 0 => {
                sink(&buffer[..read])?;
                remaining -= read;
            }
            _ => return Err(truncated),
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteError {
    Io,
    /// The status line and headers did not fit `HEAD_LIMIT`: a content type far too long.
    HeadTooLarge,
}

/// Room for the status line and headers `respond` writes.
const HEAD_LIMIT: usize = 256;

/// Writes a whole response and flushes it; the connection closes after it.
pub async fn respond(
    socket: &mut TcpSocket<'_>,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), WriteError> {
    let mut head = String::<HEAD_LIMIT>::new();
    write!(
        head,
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reason_phrase(status),
        body.len()
    )
    .map_err(|_| WriteError::HeadTooLarge)?;
    socket
        .write_all(head.as_bytes())
        .await
        .map_err(|_| WriteError::Io)?;
    socket.write_all(body).await.map_err(|_| WriteError::Io)?;
    socket.flush().await.map_err(|_| WriteError::Io)
}
