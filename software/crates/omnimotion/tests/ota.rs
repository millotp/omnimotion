use omnimotion::update::ota::{check_image_header, parse_upload, ImageError, RequestError, Upload};

const TOKEN: &str = "secret";

fn image_header(magic: u8, chip: u16) -> [u8; 24] {
    let mut header = [0; 24];
    header[0] = magic;
    header[12..14].copy_from_slice(&chip.to_le_bytes());
    header
}

#[test]
fn an_esp32s3_image_is_accepted() {
    assert_eq!(check_image_header(&image_header(0xE9, 9)), Ok(()));
}

#[test]
fn a_file_that_is_not_an_app_image_is_refused() {
    assert_eq!(
        check_image_header(&image_header(0x7F, 9)),
        Err(ImageError::NotAnImage)
    );
}

#[test]
fn an_image_built_for_another_chip_is_refused() {
    // 5 is the ESP32-C3.
    assert_eq!(
        check_image_header(&image_header(0xE9, 5)),
        Err(ImageError::WrongChip(5))
    );
}

#[test]
fn a_body_shorter_than_the_header_is_refused() {
    assert_eq!(check_image_header(&[0xE9; 23]), Err(ImageError::TooShort));
}

#[test]
fn an_authorized_upload_yields_its_length() {
    let head =
        "POST /ota HTTP/1.1\r\nHost: omnimotion\r\nX-API-Key: secret\r\nContent-Length: 1048576";
    assert_eq!(parse_upload(head, TOKEN), Ok(Upload::App(1_048_576)));
}

#[test]
fn header_names_are_case_insensitive() {
    let head = "POST /ota HTTP/1.1\r\nx-api-key: secret\r\ncontent-length: 10";
    assert_eq!(parse_upload(head, TOKEN), Ok(Upload::App(10)));
}

#[test]
fn a_web_upload_is_routed_by_path() {
    let head = "POST /web HTTP/1.1\r\nX-API-Key: secret\r\nContent-Length: 131072";
    assert_eq!(parse_upload(head, TOKEN), Ok(Upload::Web(131_072)));
}

#[test]
fn a_wrong_or_missing_key_is_unauthorized() {
    let wrong = "POST /ota HTTP/1.1\r\nX-API-Key: secreT\r\nContent-Length: 10";
    let missing = "POST /ota HTTP/1.1\r\nContent-Length: 10";
    assert_eq!(parse_upload(wrong, TOKEN), Err(RequestError::Unauthorized));
    assert_eq!(
        parse_upload(missing, TOKEN),
        Err(RequestError::Unauthorized)
    );
}

#[test]
fn an_empty_token_never_authorizes() {
    let head = "POST /ota HTTP/1.1\r\nX-API-Key: \r\nContent-Length: 10";
    assert_eq!(parse_upload(head, ""), Err(RequestError::Unauthorized));
}

#[test]
fn a_chunked_upload_is_refused_because_flash_needs_the_length_up_front() {
    let head = "POST /ota HTTP/1.1\r\nX-API-Key: secret\r\nTransfer-Encoding: chunked";
    assert_eq!(parse_upload(head, TOKEN), Err(RequestError::LengthRequired));
    let no_length = "POST /ota HTTP/1.1\r\nX-API-Key: secret";
    assert_eq!(
        parse_upload(no_length, TOKEN),
        Err(RequestError::LengthRequired)
    );
}

#[test]
fn a_repeated_length_is_malformed() {
    let head =
        "POST /ota HTTP/1.1\r\nX-API-Key: secret\r\nContent-Length: 10\r\nContent-Length: 20";
    assert_eq!(parse_upload(head, TOKEN), Err(RequestError::Malformed));
}

#[test]
fn other_paths_and_methods_are_refused() {
    let elsewhere = "POST /status HTTP/1.1\r\nX-API-Key: secret\r\nContent-Length: 10";
    let get = "GET /ota HTTP/1.1\r\nX-API-Key: secret";
    assert_eq!(parse_upload(elsewhere, TOKEN), Err(RequestError::NotFound));
    assert_eq!(
        parse_upload(get, TOKEN),
        Err(RequestError::MethodNotAllowed)
    );
}

#[test]
fn a_garbled_request_line_is_malformed() {
    assert_eq!(
        parse_upload("POST /ota", TOKEN),
        Err(RequestError::Malformed)
    );
    assert_eq!(
        parse_upload("POST /ota HTTP/2\r\nX-API-Key: secret", TOKEN),
        Err(RequestError::Malformed)
    );
    assert_eq!(
        parse_upload("POST /ota HTTP/1.1\r\nno colon here", TOKEN),
        Err(RequestError::Malformed)
    );
}
