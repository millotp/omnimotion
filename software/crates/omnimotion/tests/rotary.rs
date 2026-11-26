use omnimotion::{RotaryError, RotaryPeriod};

#[test]
fn normalizes_negative_and_multiple_revolutions() {
    let period = RotaryPeriod::new(1_000).unwrap();
    assert_eq!(period.normalize(0), 0);
    assert_eq!(period.normalize(1_000), 0);
    assert_eq!(period.normalize(3_125), 125);
    assert_eq!(period.normalize(-1), 999);
    assert_eq!(period.normalize(-2_125), 875);
}

#[test]
fn shortest_delta_wraps_in_both_directions() {
    let period = RotaryPeriod::new(1_000).unwrap();
    assert_eq!(period.shortest_delta(950, 25), 75);
    assert_eq!(period.shortest_delta(25, 950), -75);
    assert_eq!(period.shortest_delta(-2_050, 25), 75);
    assert_eq!(period.shortest_delta(3_025, 1_950), -75);
}

#[test]
fn exact_half_turn_uses_positive_direction() {
    let period = RotaryPeriod::new(1_000).unwrap();
    assert_eq!(period.shortest_delta(0, 500), 500);
    assert_eq!(period.shortest_delta(500, 0), 500);
}

#[test]
fn validates_periods_divisions_and_boundaries() {
    assert_eq!(RotaryPeriod::new(0), Err(RotaryError::ZeroPeriod));
    assert_eq!(
        RotaryPeriod::new(i64::MAX as u64 + 1),
        Err(RotaryError::Overflow)
    );

    let period = RotaryPeriod::new(1_000).unwrap();
    assert_eq!(
        period.fraction_target(1, 0),
        Err(RotaryError::ZeroDivisions)
    );
    assert_eq!(period.shortest_delta(123, 1_000), -123);
    assert_eq!(period.fraction_target(240, 240), Ok(0));
}

#[test]
fn absolute_fraction_targets_do_not_accumulate_rounding_drift() {
    let period = RotaryPeriod::new(16_003).unwrap();
    let mut raw = 0i64;

    for index in 1..240 {
        let target = period.fraction_target(index, 240).unwrap();
        raw += period.shortest_delta(raw, target);
        assert_eq!(period.normalize(raw), target);
    }
    raw += period.shortest_delta(raw, 0);

    assert_eq!(raw, period.steps());
    assert_eq!(period.normalize(raw), 0);
}
