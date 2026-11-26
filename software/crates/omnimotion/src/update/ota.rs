//! The pure half of over-the-air updates: parsing the upload request and
//! checking that the body is an ESP32-S3 app image before anything is erased.

/// ESP-IDF app images start with this byte.
pub const IMAGE_MAGIC: u8 = 0xE9;
/// `chip_id` in the extended image header, which espflash fills in.
pub const ESP32S3_CHIP_ID: u16 = 9;
const IMAGE_HEADER_LEN: usize = 24;
const CHIP_ID_OFFSET: usize = 12;

pub const UPLOAD_PATH: &str = "/ota";
pub const WEB_UPLOAD_PATH: &str = "/web";

/// What an authorized upload carries, and its length in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Upload {
    /// An app image for the inactive slot.
    App(usize),
    /// A web partition image from `pack_web.py`.
    Web(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageError {
    TooShort,
    NotAnImage,
    WrongChip(u16),
}

pub fn check_image_header(bytes: &[u8]) -> Result<(), ImageError> {
    let header = bytes.get(..IMAGE_HEADER_LEN).ok_or(ImageError::TooShort)?;
    if header[0] != IMAGE_MAGIC {
        return Err(ImageError::NotAnImage);
    }
    let chip = u16::from_le_bytes([header[CHIP_ID_OFFSET], header[CHIP_ID_OFFSET + 1]]);
    if chip != ESP32S3_CHIP_ID {
        return Err(ImageError::WrongChip(chip));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestError {
    Malformed,
    NotFound,
    MethodNotAllowed,
    Unauthorized,
    LengthRequired,
}

impl RequestError {
    pub fn status(self) -> u16 {
        match self {
            Self::Malformed => 400,
            Self::Unauthorized => 401,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::LengthRequired => 411,
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Malformed => "malformed request",
            Self::Unauthorized => "missing or wrong X-API-Key",
            Self::NotFound | Self::MethodNotAllowed => {
                "only POST /ota and POST /web are served here"
            }
            Self::LengthRequired => "Content-Length required",
        }
    }
}

/// Parses the head of a `POST /ota` or `POST /web` request, without the blank
/// line that ends it, and returns the target with the body length. The image is
/// streamed straight to flash, so chunked bodies are refused: the length has to
/// be known before the first erase.
pub fn parse_upload(head: &str, token: &str) -> Result<Upload, RequestError> {
    let mut lines = head.split("\r\n");
    let mut request_line = lines
        .next()
        .ok_or(RequestError::Malformed)?
        .split_ascii_whitespace();
    let method = request_line.next().ok_or(RequestError::Malformed)?;
    let path = request_line.next().ok_or(RequestError::Malformed)?;
    let version = request_line.next().ok_or(RequestError::Malformed)?;
    if request_line.next().is_some() || !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err(RequestError::Malformed);
    }

    let mut length = None;
    let mut authorized = false;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(RequestError::Malformed)?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(RequestError::Malformed);
            }
            length = Some(value.parse().map_err(|_| RequestError::Malformed)?);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(RequestError::LengthRequired);
        } else if name.eq_ignore_ascii_case("x-api-key") {
            authorized = !token.is_empty() && constant_time_equal(value, token);
        }
    }

    let target = match path {
        UPLOAD_PATH => Upload::App,
        WEB_UPLOAD_PATH => Upload::Web,
        _ => return Err(RequestError::NotFound),
    };
    if method != "POST" {
        return Err(RequestError::MethodNotAllowed);
    }
    if !authorized {
        return Err(RequestError::Unauthorized);
    }
    let length = length.ok_or(RequestError::LengthRequired)?;
    Ok(target(length))
}

/// Takes as long for a near miss as for a wild guess.
fn constant_time_equal(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0, |difference, (x, y)| difference | (x ^ y))
            == 0
}
