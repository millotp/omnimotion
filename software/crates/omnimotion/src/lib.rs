#![no_std]

pub mod devices;
pub mod machine;
pub mod motion;
pub mod net;
pub mod update;

/// Axes M0-M2. All three TMC2209 channels share one UART, addressed by their
/// MS1/MS2 straps.
pub const AXES: usize = 3;

/// One bit per axis, bit `n` being axis `n`. The step ISR turns these straight
/// into a single GPIO register write, so they are the engine's native form for
/// anything per-axis and boolean.
pub type AxisMask = u8;

const _: () = assert!(AXES <= AxisMask::BITS as usize);

pub use devices::endstop::{EndstopBank, EndstopConfig, EndstopKind, EndstopRole, ENDSTOPS};
pub use devices::power::{PowerSource, Rails, Volts};
pub use machine::control::{
    CommandCompletion, CommandError, CommandRequest, ControlClient, ControlPlane, MachineCommand,
    MachineControl, RequestId, StandardCommand, StatusSnapshot, SubmitError, KEPT_COMPLETIONS,
};
pub use machine::lifecycle::{JobProgress, Lifecycle, LifecycleError, LifecycleState};
pub use machine::persistence::{
    decode_record, encode_record, encoded_len, prepare_write, select_latest, DecodedRecord,
    EncodeError, LoadError, PendingWrite, RecordKind, RecordMeta, SelectedRecord, Slot, WriteGate,
    WriteRefusal,
};
pub use motion::abort::{generation_is_acknowledged, AbortGenerations, AbortState, AbortTicket};
pub use motion::engine::{EngineError, StepEngine, Tick, TICK_HZ};
pub use motion::planner::{nco_increment, MoveStart, PlanError, Planner, Segment};
pub use motion::profile::{Limits, LimitsError, Profile, Trapezoid};
pub use motion::rotary::{RotaryError, RotaryPeriod};
pub use motion::units::{group_limits, steps_per_mm, AxisConfig, UnitError};
