use omnimotion::{EndstopBank, EndstopConfig, EndstopRole, ENDSTOPS};

/// Nothing triggered: every input idles high through its pull-up.
const IDLE: u8 = 0xFF;

fn bank(config: EndstopConfig) -> EndstopBank {
    EndstopBank::new([config; ENDSTOPS])
}

fn low(endstop: usize) -> u8 {
    IDLE & !(1 << endstop)
}

#[test]
fn idle_inputs_never_trigger() {
    let mut switches = bank(EndstopConfig::SWITCH);
    for _ in 0..10 {
        assert_eq!(switches.sample(IDLE), 0);
    }
    assert!(!switches.settling());
}

#[test]
fn a_switch_needs_three_agreeing_samples() {
    let mut switches = bank(EndstopConfig::SWITCH);

    assert_eq!(switches.sample(low(0)), 0);
    assert!(switches.settling());
    assert_eq!(switches.sample(low(0)), 0);
    assert_eq!(switches.sample(low(0)), 1 << 0);
}

#[test]
fn a_hall_sensor_trips_on_the_first_sample() {
    let mut halls = bank(EndstopConfig::HALL);

    assert_eq!(halls.sample(low(3)), 1 << 3);
}

#[test]
fn bounce_restarts_the_switch_debounce() {
    let mut switches = bank(EndstopConfig::SWITCH);

    switches.sample(low(1));
    switches.sample(low(1));
    assert_eq!(switches.sample(IDLE), 0);
    assert!(!switches.settling());

    // The count restarted, so it takes three more samples.
    assert_eq!(switches.sample(low(1)), 0);
    assert_eq!(switches.sample(low(1)), 0);
    assert_eq!(switches.sample(low(1)), 1 << 1);
}

#[test]
fn unused_connectors_stay_quiet_whatever_the_pin_reads() {
    let mut unused = bank(EndstopConfig::UNUSED);

    for _ in 0..10 {
        assert_eq!(unused.sample(0x00), 0);
    }
    assert_eq!(unused.triggered_now(0x00), 0);
    assert!(!unused.settling());
}

/// Normally-closed wiring idles low, so a cut cable reads as triggered.
#[test]
fn inverted_polarity_trips_when_the_input_goes_high() {
    let closed = EndstopConfig {
        active_low: false,
        ..EndstopConfig::SWITCH
    };
    let mut switches = bank(closed);

    assert_eq!(switches.sample(0x00), 0);
    assert_eq!(switches.sample(0x00), 0);
    assert_eq!(switches.sample(0x00), 0);
    assert_eq!(switches.triggered_now(0x00), 0);

    assert_eq!(switches.sample(IDLE), 0);
    assert_eq!(switches.sample(IDLE), 0);
    assert_eq!(switches.sample(IDLE), 0b0011_1111);
}

#[test]
fn each_endstop_debounces_on_its_own_type() {
    let mut mixed = EndstopBank::new([
        EndstopConfig::HALL,
        EndstopConfig::SWITCH,
        EndstopConfig::UNUSED,
        EndstopConfig::UNUSED,
        EndstopConfig::UNUSED,
        EndstopConfig::UNUSED,
    ]);

    // Both go low together; only the hall sensor is believed straight away.
    let both = IDLE & !0b11;
    assert_eq!(mixed.sample(both), 1 << 0);
    assert_eq!(mixed.sample(both), 1 << 0);
    assert_eq!(mixed.sample(both), 0b11);
}

#[test]
fn reset_clears_a_part_way_debounce() {
    let mut switches = bank(EndstopConfig::SWITCH);

    switches.sample(low(4));
    switches.sample(low(4));
    assert!(switches.settling());

    switches.reset();
    assert!(!switches.settling());
    assert_eq!(switches.sample(low(4)), 0);
}

#[test]
fn config_is_readable_back_and_bounded() {
    let bank = bank(EndstopConfig::HALL);

    assert_eq!(bank.config(0), Some(EndstopConfig::HALL));
    assert_eq!(bank.config(ENDSTOPS), None);
}

#[test]
fn the_shipped_configs_carry_the_role_their_name_promises() {
    assert_eq!(EndstopConfig::SWITCH.role, EndstopRole::Limit);
    assert_eq!(EndstopConfig::HALL.role, EndstopRole::Limit);
    assert_eq!(EndstopConfig::SWITCH_REFERENCE.role, EndstopRole::Reference);
    assert_eq!(EndstopConfig::HALL_REFERENCE.role, EndstopRole::Reference);

    // A reference only differs in role: it must debounce like its sensor type.
    assert_eq!(
        EndstopConfig::SWITCH_REFERENCE.kind,
        EndstopConfig::SWITCH.kind
    );
    assert_eq!(EndstopConfig::HALL_REFERENCE.kind, EndstopConfig::HALL.kind);
}

#[test]
fn role_mask_splits_the_limits_from_the_references() {
    let bank = EndstopBank::new([
        EndstopConfig::SWITCH_REFERENCE, // E0: nail zero index
        EndstopConfig::SWITCH,           // E1: real limit
        EndstopConfig::HALL_REFERENCE,   // E2: arm retract datum
        EndstopConfig::UNUSED,
        EndstopConfig::UNUSED,
        EndstopConfig::UNUSED,
    ]);

    assert_eq!(bank.role_mask(EndstopRole::Reference), 0b0000_0101);
    assert_eq!(bank.role_mask(EndstopRole::Limit), 0b0000_0010);
}

/// An unfitted connector belongs to no role, so a stray low cannot reach the
/// supervisor through `role_mask` either.
#[test]
fn unused_connectors_are_in_no_role() {
    let bank = bank(EndstopConfig::UNUSED);

    assert_eq!(bank.role_mask(EndstopRole::Limit), 0);
    assert_eq!(bank.role_mask(EndstopRole::Reference), 0);
}

/// The whole point of the reference role: a job may cross it repeatedly, so
/// `sample` still reports it and the caller decides it is not a fault.
#[test]
fn references_debounce_and_report_like_limits() {
    let mut mixed = EndstopBank::new([
        EndstopConfig::SWITCH_REFERENCE,
        EndstopConfig::SWITCH,
        EndstopConfig::UNUSED,
        EndstopConfig::UNUSED,
        EndstopConfig::UNUSED,
        EndstopConfig::UNUSED,
    ]);

    let reference_only = low(0);
    assert_eq!(mixed.sample(reference_only), 0);
    assert_eq!(mixed.sample(reference_only), 0);
    assert_eq!(mixed.sample(reference_only), 1 << 0);

    // Nothing in the Limit role triggered, which is what spares the fault.
    assert_eq!(
        mixed.sample(reference_only) & mixed.role_mask(EndstopRole::Limit),
        0
    );
}
