use omnimotion::{
    decode_record, encode_record, encoded_len, prepare_write, select_latest, EncodeError,
    LifecycleState, LoadError, RecordKind, RecordMeta, Slot, WriteGate, WriteRefusal,
};
use serde::{Deserialize, Serialize};

fn record(kind: RecordKind, schema: u16, generation: u64, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; encoded_len(payload.len()).unwrap()];
    let len = encode_record(
        RecordMeta {
            kind,
            schema_version: schema,
            generation,
        },
        payload,
        &mut bytes,
    )
    .unwrap()
    .len();
    bytes.truncate(len);
    bytes
}

#[test]
fn valid_record_round_trips() {
    let bytes = record(RecordKind::SETTINGS, 2, 7, b"settings");
    let decoded = decode_record(&bytes, RecordKind::SETTINGS, 2).unwrap();
    assert_eq!(decoded.meta.generation, 7);
    assert_eq!(decoded.payload, b"settings");

    let mut too_small = [0; 4];
    assert_eq!(
        encode_record(
            RecordMeta {
                kind: RecordKind::SETTINGS,
                schema_version: 1,
                generation: 1,
            },
            b"x",
            &mut too_small
        ),
        Err(EncodeError::BufferTooSmall)
    );
}

#[test]
fn missing_corrupt_wrong_kind_and_unsupported_are_distinct() {
    assert_eq!(
        select_latest(None, None, RecordKind::SETTINGS, 1),
        Err(LoadError::Missing)
    );

    let mut corrupt = record(RecordKind::SETTINGS, 1, 1, b"value");
    corrupt[23] ^= 0x80;
    assert_eq!(
        select_latest(Some(&corrupt), None, RecordKind::SETTINGS, 1),
        Err(LoadError::Corrupt)
    );

    let checkpoint = record(RecordKind::CHECKPOINT, 1, 1, b"value");
    assert_eq!(
        select_latest(Some(&checkpoint), None, RecordKind::SETTINGS, 1),
        Err(LoadError::WrongKind {
            found: RecordKind::CHECKPOINT,
            expected: RecordKind::SETTINGS
        })
    );

    let future = record(RecordKind::SETTINGS, 3, 1, b"value");
    assert_eq!(
        select_latest(Some(&future), None, RecordKind::SETTINGS, 2),
        Err(LoadError::UnsupportedSchema {
            found: 3,
            supported: 2
        })
    );
}

#[test]
fn torn_new_slot_recovers_the_previous_complete_generation() {
    let old = record(RecordKind::CHECKPOINT, 1, 10, b"old");
    let new = record(RecordKind::CHECKPOINT, 1, 11, b"new");
    let torn = &new[..new.len() - 2];

    let selected = select_latest(Some(&old), Some(torn), RecordKind::CHECKPOINT, 1).unwrap();
    assert_eq!(selected.slot, Slot::A);
    assert_eq!(selected.record.meta.generation, 10);
    assert_eq!(selected.record.payload, b"old");

    let pending = prepare_write(Some(&old), Some(torn), RecordKind::CHECKPOINT, 1).unwrap();
    assert_eq!(pending.slot, Slot::B);
    assert_eq!(pending.generation, 11);
}

#[test]
fn newest_complete_unsupported_schema_is_not_silently_replaced() {
    let old = record(RecordKind::SETTINGS, 1, 3, b"old");
    let future = record(RecordKind::SETTINGS, 2, 4, b"future");
    assert_eq!(
        prepare_write(Some(&old), Some(&future), RecordKind::SETTINGS, 1),
        Err(LoadError::UnsupportedSchema {
            found: 2,
            supported: 1
        })
    );
}

#[test]
fn a_migrated_record_replaces_the_older_schema_in_the_other_slot() {
    let v1 = record(RecordKind::SETTINGS, 1, 5, b"v1");
    assert_eq!(
        select_latest(Some(&v1), None, RecordKind::SETTINGS, 2),
        Err(LoadError::UnsupportedSchema {
            found: 1,
            supported: 2
        })
    );

    let pending = prepare_write(Some(&v1), None, RecordKind::SETTINGS, 2).unwrap();
    assert_eq!(pending.slot, Slot::B);
    assert_eq!(pending.generation, 6);

    let v2 = record(RecordKind::SETTINGS, 2, pending.generation, b"v2");
    let selected = select_latest(Some(&v1), Some(&v2), RecordKind::SETTINGS, 2).unwrap();
    assert_eq!(selected.record.payload, b"v2");
}

#[test]
fn generation_exhaustion_is_reported_instead_of_wrapping() {
    let latest = record(RecordKind::CHECKPOINT, 1, u64::MAX, b"value");
    assert_eq!(
        prepare_write(Some(&latest), None, RecordKind::CHECKPOINT, 1),
        Err(LoadError::GenerationExhausted)
    );
}

#[derive(Serialize)]
struct OldSettings {
    speed: u32,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct NewSettings {
    speed: u32,
    acceleration: u32,
}

#[test]
fn changed_postcard_shape_is_not_interpreted_as_valid() {
    let payload = postcard::to_stdvec(&OldSettings { speed: 42 }).unwrap();
    let bytes = record(RecordKind::SETTINGS, 1, 1, &payload);
    let decoded = decode_record(&bytes, RecordKind::SETTINGS, 1).unwrap();
    assert!(postcard::from_bytes::<NewSettings>(decoded.payload).is_err());
}

#[test]
fn write_gate_rejects_every_unsafe_condition() {
    let safe = WriteGate {
        motion_drained: true,
        abort_pending: false,
        lifecycle: LifecycleState::Paused,
        safe_boundary: true,
    };
    assert_eq!(safe.check(), Ok(()));
    assert_eq!(
        WriteGate {
            motion_drained: false,
            ..safe
        }
        .check(),
        Err(WriteRefusal::MotionActive)
    );
    assert_eq!(
        WriteGate {
            abort_pending: true,
            ..safe
        }
        .check(),
        Err(WriteRefusal::AbortPending)
    );
    assert_eq!(
        WriteGate {
            lifecycle: LifecycleState::Running,
            ..safe
        }
        .check(),
        Err(WriteRefusal::UnsafeLifecycle)
    );
    assert_eq!(
        WriteGate {
            safe_boundary: false,
            ..safe
        }
        .check(),
        Err(WriteRefusal::UnsafeLifecycle)
    );
}
