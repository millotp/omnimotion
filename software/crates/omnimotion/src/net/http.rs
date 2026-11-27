//! The parts of HTTP/1.x the board's servers share: finding and parsing a
//! request head, and the status line of a response. Bodies always come with a
//! length, since every server here either streams them to flash or reads them
//! into a buffer of known size.

/// The longest method and path a request may have.
pub const METHOD_LIMIT: usize = 8;
pub const PATH_LIMIT: usize = 64;

/// A parsed request head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head<'a> {
    pub method: &'a str,
    /// Without the query.
    pub path: &'a str,
    pub content_length: Option<usize>,
    /// `X-API-Key` matched the token.
    pub authorized: bool,
}

/// Why a request got no further than its head; each is answered as is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// The connection failed or timed out, so nothing can be answered.
    Io,
    Malformed,
    /// The head did not fit the server's buffer.
    HeadTooLarge,
    Unauthorized,
    NotFound,
    MethodNotAllowed,
    /// A chunked body, or a POST without `Content-Length`.
    LengthRequired,
    PayloadTooLarge,
    /// Another large body is being received.
    Busy,
}

impl RequestError {
    pub fn status(self) -> u16 {
        match self {
            Self::Io | Self::Malformed => 400,
            Self::Unauthorized => 401,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::LengthRequired => 411,
            Self::PayloadTooLarge => 413,
            Self::HeadTooLarge => 431,
            Self::Busy => 503,
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Io | Self::Malformed => "malformed request",
            Self::HeadTooLarge => "request headers too large",
            Self::Unauthorized => "missing or wrong X-API-Key",
            Self::NotFound => "not found",
            Self::MethodNotAllowed => "method not allowed",
            Self::LengthRequired => "Content-Length required",
            Self::PayloadTooLarge => "request body too large",
            Self::Busy => "another upload is in progress",
        }
    }
}

impl Head<'_> {
    /// The body's length: a POST must give one, other requests default to none.
    pub fn body_length(&self) -> Result<usize, RequestError> {
        match (self.method, self.content_length) {
            ("POST", None) => Err(RequestError::LengthRequired),
            (_, length) => Ok(length.unwrap_or(0)),
        }
    }
}

/// The length of the head at the start of `buffer`, through the blank line
/// that ends it, once it is all there.
pub fn head_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|start| start + 4)
}

/// Parses a request head, with or without its closing blank line. `X-API-Key`
/// is compared with `token` in constant time, and an empty token authorizes
/// nothing.
pub fn parse_head<'a>(head: &'a [u8], token: &[u8]) -> Result<Head<'a>, RequestError> {
    use RequestError::Malformed;
    let text = core::str::from_utf8(head).map_err(|_| Malformed)?;
    let mut lines = text.split("\r\n");
    let mut request_line = lines.next().ok_or(Malformed)?.split_ascii_whitespace();
    let method = request_line.next().ok_or(Malformed)?;
    let raw_target = request_line.next().ok_or(Malformed)?;
    let version = request_line.next().ok_or(Malformed)?;
    if request_line.next().is_some()
        || !matches!(version, "HTTP/1.0" | "HTTP/1.1")
        || method.len() > METHOD_LIMIT
    {
        return Err(Malformed);
    }
    let path = normalized_path(raw_target).ok_or(Malformed)?;
    if path.len() > PATH_LIMIT {
        return Err(Malformed);
    }
    let mut content_length = None;
    let mut authorized = false;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(Malformed)?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(Malformed);
            }
            content_length = Some(value.parse().map_err(|_| Malformed)?);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(RequestError::LengthRequired);
        } else if name.eq_ignore_ascii_case("x-api-key") {
            authorized = !token.is_empty() && constant_time_equal(value.as_bytes(), token);
        }
    }
    Ok(Head {
        method,
        path,
        content_length,
        authorized,
    })
}

/// The path of a request target without its query, or `None` for a target
/// that could climb out of the served tree or hide that it does.
pub fn normalized_path(target: &str) -> Option<&str> {
    if !target.starts_with('/')
        || target.contains('%')
        || target.contains("..")
        || target.contains("//")
    {
        return None;
    }
    target.split('?').next()
}

/// Takes as long for a near miss as for a wild guess.
pub fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= usize::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

pub fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        422 => "Unprocessable Content",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    }
}
