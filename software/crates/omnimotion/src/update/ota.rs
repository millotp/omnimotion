//! The pure half of over-the-air updates: parsing the upload request and
//! checking that the body is an ESP32-S3 app image before anything is erased.

use crate::net::http::{parse_head, HeadError};

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

/// Parses the head of a `POST /ota` or `POST /web` request and returns the
/// target with the body length. The image is streamed straight to flash, so
/// chunked bodies are refused: the length has to be known before the first
/// erase.
pub fn parse_upload(head: &str, token: &str) -> Result<Upload, RequestError> {
    let head = parse_head(head.as_bytes(), token.as_bytes()).map_err(|error| match error {
        HeadError::Malformed => RequestError::Malformed,
        HeadError::LengthRequired => RequestError::LengthRequired,
    })?;
    let target = match head.path {
        UPLOAD_PATH => Upload::App,
        WEB_UPLOAD_PATH => Upload::Web,
        _ => return Err(RequestError::NotFound),
    };
    if head.method != "POST" {
        return Err(RequestError::MethodNotAllowed);
    }
    if !head.authorized {
        return Err(RequestError::Unauthorized);
    }
    let length = head
        .body_length()
        .map_err(|_| RequestError::LengthRequired)?;
    Ok(target(length))
}
