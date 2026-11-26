use omnimotion::update::web::{is_nvs_page, PAGE_LEN};

fn page(state: u32, version: u8) -> Vec<u8> {
    let mut page = vec![0xFF; PAGE_LEN];
    page[0..4].copy_from_slice(&state.to_le_bytes());
    page[8] = version;
    page
}

#[test]
fn a_full_or_active_page_with_the_nvs_version_is_accepted() {
    assert!(is_nvs_page(&page(0xFFFF_FFFC, 0xFE)));
    assert!(is_nvs_page(&page(0xFFFF_FFFE, 0xFE)));
}

#[test]
fn an_erased_page_is_accepted() {
    assert!(is_nvs_page(&vec![0xFF; PAGE_LEN]));
}

#[test]
fn a_page_of_the_wrong_length_is_refused() {
    assert!(!is_nvs_page(&vec![0xFF; PAGE_LEN - 1]));
    assert!(!is_nvs_page(&vec![0xFF; PAGE_LEN + 1]));
}

#[test]
fn a_written_page_with_another_version_is_refused() {
    assert!(!is_nvs_page(&page(0xFFFF_FFFC, 0x01)));
}

#[test]
fn a_page_with_an_unknown_state_is_refused() {
    // 0 is the state NVS gives a corrupt page.
    assert!(!is_nvs_page(&page(0, 0xFE)));
    assert!(!is_nvs_page(&page(0x1234_5678, 0xFE)));
}
