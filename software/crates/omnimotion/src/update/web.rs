//! The pure half of web-partition updates: checking each uploaded page is an
//! NVS page before its sector is erased.
//!
//! `tools/pack_web.py` writes the partition as NVS pages, one flash sector each.
//! A page is either written, FULL or ACTIVE with version `0xFE`, or still erased.

/// One NVS page, which is one flash sector.
pub const PAGE_LEN: usize = 4096;

const STATE_EMPTY: u32 = 0xFFFF_FFFF;
const STATE_FULL: u32 = 0xFFFF_FFFC;
const STATE_ACTIVE: u32 = 0xFFFF_FFFE;
const VERSION_OFFSET: usize = 8;
const VERSION: u8 = 0xFE;

pub fn is_nvs_page(page: &[u8]) -> bool {
    if page.len() != PAGE_LEN {
        return false;
    }
    let state = u32::from_le_bytes([page[0], page[1], page[2], page[3]]);
    match state {
        STATE_EMPTY => true,
        STATE_FULL | STATE_ACTIVE => page[VERSION_OFFSET] == VERSION,
        _ => false,
    }
}
