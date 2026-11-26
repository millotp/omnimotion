use omnimotion::{PowerSource, Rails, Volts};

/// What the ADC pin sees for a given VMOTOR: divided by 11.
fn pin_mv(volts: f32) -> i32 {
    (volts / 11.0 * 1_000.0) as i32
}

fn rails(motor: f32) -> Rails {
    Rails::from_pin_mv(pin_mv(motor))
}

#[test]
fn the_pin_reading_scales_back_to_the_rail_it_came_from() {
    let rails = rails(24.0);

    assert!(
        (23_950..=24_050).contains(&rails.motor_mv),
        "24 V read back as {} mV",
        rails.motor_mv
    );
}

#[test]
fn the_documented_divider_output_holds() {
    // Sheet 2 states 30 V gives 2.727 V at the monitor pin.
    assert_eq!(pin_mv(30.0), 2_727);
}

#[test]
fn no_motor_supply_reads_as_running_on_usb() {
    // USB alone: the buck is dark and only ADC offset shows on the pin.
    assert_eq!(rails(0.0).source(), PowerSource::UsbOnly);
    assert_eq!(rails(0.3).source(), PowerSource::UsbOnly);
}

#[test]
fn negative_offset_never_reports_a_negative_rail() {
    let rails = Rails::from_pin_mv(-12);

    assert_eq!(rails.motor_mv, 0);
    assert_eq!(rails.source(), PowerSource::UsbOnly);
}

#[test]
fn the_supported_input_range_reads_as_bulk() {
    for volts in [12.0, 19.0, 24.0] {
        assert_eq!(
            rails(volts).source(),
            PowerSource::Bulk,
            "{volts} V is inside the supported range"
        );
    }
}

#[test]
fn a_supply_outside_twelve_to_twentyfour_is_flagged() {
    // 5 V into the barrel jack: present, but the buck cannot make 5 V from it.
    assert_eq!(rails(5.0).source(), PowerSource::BulkOutOfRange);
    // A 36 V brick, which the 27.4 V overvoltage trip exists to survive.
    assert_eq!(rails(30.0).source(), PowerSource::BulkOutOfRange);
}

#[test]
fn millivolts_read_out_as_volts_without_losing_the_leading_zero() {
    assert_eq!(Volts(23_991).to_string(), "23.99 V");
    assert_eq!(Volts(5_007).to_string(), "5.00 V");
    assert_eq!(Volts(5_010).to_string(), "5.01 V");
    assert_eq!(Volts(12_000).to_string(), "12.00 V");
    assert_eq!(Volts(480).to_string(), "0.48 V");
    assert_eq!(Volts(0).to_string(), "0.00 V");
}
