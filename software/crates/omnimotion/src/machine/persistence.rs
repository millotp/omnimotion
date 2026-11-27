use crc::{Crc, CRC_32_ISO_HDLC};

use crate::LifecycleState;

/// The NVS limit on a key or namespace, excluding the null terminator.
pub const MAX_NAME_LEN: usize = 15;

/// Whether NVS can store under `namespace` and `key`; `esp-nvs` panics on longer names.
pub fn names_fit(namespace: &str, key: &str) -> bool {
    namespace.len() <= MAX_NAME_LEN && key.len() <= MAX_NAME_LEN
}

const MAGIC: [u8; 4] = *b"OMNI";
const HEADER_LEN: usize = 24;
const CRC_OFFSET: usize = 20;
const CRC32: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordKind(pub u16);

impl RecordKind {
    pub const SETTINGS: Self = Self(1);
    pub const CHECKPOINT: Self = Self(2);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordMeta {
    pub kind: RecordKind,
    pub schema_version: u16,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeError {
    PayloadTooLarge,
    BufferTooSmall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    Missing,
    Corrupt,
    WrongKind {
        found: RecordKind,
        expected: RecordKind,
    },
    UnsupportedSchema {
        found: u16,
        supported: u16,
    },
    GenerationExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    A,
    B,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedRecord<'a> {
    pub meta: RecordMeta,
    pub payload: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedRecord<'a> {
    pub slot: Slot,
    pub record: DecodedRecord<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingWrite {
    pub slot: Slot,
    pub generation: u64,
}

pub const fn encoded_len(payload_len: usize) -> Option<usize> {
    HEADER_LEN.checked_add(payload_len)
}

pub fn encode_record<'a>(
    meta: RecordMeta,
    payload: &[u8],
    output: &'a mut [u8],
) -> Result<&'a [u8], EncodeError> {
    let payload_len = u32::try_from(payload.len()).map_err(|_| EncodeError::PayloadTooLarge)?;
    let total = encoded_len(payload.len()).ok_or(EncodeError::PayloadTooLarge)?;
    if output.len() < total {
        return Err(EncodeError::BufferTooSmall);
    }

    output[..4].copy_from_slice(&MAGIC);
    output[4..6].copy_from_slice(&meta.kind.0.to_le_bytes());
    output[6..8].copy_from_slice(&meta.schema_version.to_le_bytes());
    output[8..16].copy_from_slice(&meta.generation.to_le_bytes());
    output[16..20].copy_from_slice(&payload_len.to_le_bytes());
    output[CRC_OFFSET..HEADER_LEN].fill(0);
    output[HEADER_LEN..total].copy_from_slice(payload);

    let mut digest = CRC32.digest();
    digest.update(&output[..CRC_OFFSET]);
    digest.update(payload);
    output[CRC_OFFSET..HEADER_LEN].copy_from_slice(&digest.finalize().to_le_bytes());
    Ok(&output[..total])
}

pub fn decode_record(
    bytes: &[u8],
    expected_kind: RecordKind,
    supported_schema: u16,
) -> Result<DecodedRecord<'_>, LoadError> {
    let record = decode_complete(bytes)?;
    if record.meta.kind != expected_kind {
        return Err(LoadError::WrongKind {
            found: record.meta.kind,
            expected: expected_kind,
        });
    }
    if record.meta.schema_version != supported_schema {
        return Err(LoadError::UnsupportedSchema {
            found: record.meta.schema_version,
            supported: supported_schema,
        });
    }
    Ok(record)
}

pub fn select_latest<'a>(
    slot_a: Option<&'a [u8]>,
    slot_b: Option<&'a [u8]>,
    expected_kind: RecordKind,
    supported_schema: u16,
) -> Result<SelectedRecord<'a>, LoadError> {
    let selected = select_complete(slot_a, slot_b, expected_kind)?;
    if selected.record.meta.schema_version != supported_schema {
        return Err(LoadError::UnsupportedSchema {
            found: selected.record.meta.schema_version,
            supported: supported_schema,
        });
    }
    Ok(selected)
}

/// An older stored schema may be replaced, which is how a migration lands; a
/// newer one is refused so older firmware cannot clobber it.
pub fn prepare_write(
    slot_a: Option<&[u8]>,
    slot_b: Option<&[u8]>,
    expected_kind: RecordKind,
    schema_version: u16,
) -> Result<PendingWrite, LoadError> {
    match select_complete(slot_a, slot_b, expected_kind) {
        Ok(selected) if selected.record.meta.schema_version > schema_version => {
            Err(LoadError::UnsupportedSchema {
                found: selected.record.meta.schema_version,
                supported: schema_version,
            })
        }
        Ok(selected) => Ok(PendingWrite {
            slot: match selected.slot {
                Slot::A => Slot::B,
                Slot::B => Slot::A,
            },
            generation: selected
                .record
                .meta
                .generation
                .checked_add(1)
                .ok_or(LoadError::GenerationExhausted)?,
        }),
        Err(LoadError::Missing) => Ok(PendingWrite {
            slot: Slot::A,
            generation: 1,
        }),
        Err(error) => Err(error),
    }
}

fn select_complete<'a>(
    slot_a: Option<&'a [u8]>,
    slot_b: Option<&'a [u8]>,
    expected_kind: RecordKind,
) -> Result<SelectedRecord<'a>, LoadError> {
    let a = slot_a.and_then(|bytes| decode_complete(bytes).ok());
    let b = slot_b.and_then(|bytes| decode_complete(bytes).ok());
    let selected = match (a, b) {
        (Some(a), Some(b)) if b.meta.generation > a.meta.generation => SelectedRecord {
            slot: Slot::B,
            record: b,
        },
        (Some(a), _) => SelectedRecord {
            slot: Slot::A,
            record: a,
        },
        (None, Some(b)) => SelectedRecord {
            slot: Slot::B,
            record: b,
        },
        (None, None) => {
            return if slot_a.is_none() && slot_b.is_none() {
                Err(LoadError::Missing)
            } else {
                Err(LoadError::Corrupt)
            };
        }
    };
    if selected.record.meta.kind != expected_kind {
        return Err(LoadError::WrongKind {
            found: selected.record.meta.kind,
            expected: expected_kind,
        });
    }
    Ok(selected)
}

fn decode_complete(bytes: &[u8]) -> Result<DecodedRecord<'_>, LoadError> {
    if bytes.len() < HEADER_LEN || bytes[..4] != MAGIC {
        return Err(LoadError::Corrupt);
    }
    let kind = RecordKind(u16::from_le_bytes(
        bytes[4..6].try_into().map_err(|_| LoadError::Corrupt)?,
    ));
    let schema_version =
        u16::from_le_bytes(bytes[6..8].try_into().map_err(|_| LoadError::Corrupt)?);
    let generation = u64::from_le_bytes(bytes[8..16].try_into().map_err(|_| LoadError::Corrupt)?);
    let payload_len =
        u32::from_le_bytes(bytes[16..20].try_into().map_err(|_| LoadError::Corrupt)?) as usize;
    let total = encoded_len(payload_len).ok_or(LoadError::Corrupt)?;
    if bytes.len() != total {
        return Err(LoadError::Corrupt);
    }
    let stored_crc = u32::from_le_bytes(
        bytes[CRC_OFFSET..HEADER_LEN]
            .try_into()
            .map_err(|_| LoadError::Corrupt)?,
    );
    let payload = &bytes[HEADER_LEN..];
    let mut digest = CRC32.digest();
    digest.update(&bytes[..CRC_OFFSET]);
    digest.update(payload);
    if digest.finalize() != stored_crc {
        return Err(LoadError::Corrupt);
    }
    Ok(DecodedRecord {
        meta: RecordMeta {
            kind,
            schema_version,
            generation,
        },
        payload,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteGate {
    pub motion_drained: bool,
    pub abort_pending: bool,
    pub lifecycle: LifecycleState,
    pub safe_boundary: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteRefusal {
    MotionActive,
    AbortPending,
    UnsafeLifecycle,
}

impl WriteGate {
    pub fn check(self) -> Result<(), WriteRefusal> {
        if !self.motion_drained {
            return Err(WriteRefusal::MotionActive);
        }
        if self.abort_pending {
            return Err(WriteRefusal::AbortPending);
        }
        if !self.safe_boundary
            || !matches!(
                self.lifecycle,
                LifecycleState::Idle | LifecycleState::Paused
            )
        {
            return Err(WriteRefusal::UnsafeLifecycle);
        }
        Ok(())
    }
}
