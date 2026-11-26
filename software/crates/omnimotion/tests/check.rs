use omnimotion::devices::check::{
    read_endstops, BoardReport, DriverCheck, DriverResult, DriverSamples, EndstopReading,
    Temperature,
};
use omnimotion::{EndstopConfig, Rails, ENDSTOPS};
use tmc2209::reg;

const ENN: u32 = 1 << 0;
const MS1: u32 = 1 << 2;
const MS2: u32 = 1 << 3;
const DIAG: u32 = 1 << 4;
const STEP: u32 = 1 << 7;
const DIR: u32 = 1 << 9;

/// What a correctly wired driver at `address` reports, outputs disabled.
fn wired(address: u8) -> DriverSamples {
    let straps = if address & 1 != 0 { MS1 } else { 0 } | if address & 2 != 0 { MS2 } else { 0 };
    DriverSamples {
        low: reg::IOIN(ENN | straps),
        high: reg::IOIN(ENN | straps | STEP | DIR),
        mcu_diag: [false, false],
        global: reg::GSTAT(0),
        status: reg::DRV_STATUS(0),
    }
}

#[test]
fn a_correctly_wired_driver_passes_at_every_address() {
    for address in 0..3 {
        let check = DriverCheck::judge(address, &wired(address));
        assert!(check.passed(), "address {address}: {check:?}");
    }
}

#[test]
fn a_step_trace_that_never_rises_fails_only_step() {
    let mut samples = wired(0);
    samples.high = reg::IOIN(samples.high.0 & !STEP);
    let check = DriverCheck::judge(0, &samples);
    assert!(!check.step);
    assert!(check.dir && check.diag && check.straps && check.enable_off);
}

#[test]
fn a_dir_trace_stuck_high_fails_dir() {
    let mut samples = wired(1);
    samples.low = reg::IOIN(samples.low.0 | DIR);
    assert!(!DriverCheck::judge(1, &samples).dir);
}

#[test]
fn enn_reading_active_while_disabled_fails() {
    let mut samples = wired(0);
    samples.low = reg::IOIN(samples.low.0 & !ENN);
    samples.high = reg::IOIN(samples.high.0 & !ENN);
    assert!(!DriverCheck::judge(0, &samples).enable_off);
}

#[test]
fn straps_that_disagree_with_the_answering_address_fail() {
    // Strapped as 1 but answering on 2 means the address decode is crossed.
    let samples = wired(1);
    assert!(!DriverCheck::judge(2, &samples).straps);
}

#[test]
fn diag_must_agree_on_both_sides() {
    let mut samples = wired(0);
    samples.mcu_diag = [true, true];
    assert!(!DriverCheck::judge(0, &samples).diag);

    samples.low = reg::IOIN(samples.low.0 | DIAG);
    samples.high = reg::IOIN(samples.high.0 | DIAG);
    assert!(DriverCheck::judge(0, &samples).diag);
}

#[test]
fn driver_health_flags_fail_the_check() {
    let mut samples = wired(0);
    samples.global = reg::GSTAT(1 << 2);
    assert!(!DriverCheck::judge(0, &samples).charge_pump);

    let mut samples = wired(0);
    samples.status = reg::DRV_STATUS(1 << 0);
    let check = DriverCheck::judge(0, &samples);
    assert_eq!(check.temperature, Temperature::Warning);
    assert!(!check.passed());

    samples.status = reg::DRV_STATUS(0b11);
    assert_eq!(
        DriverCheck::judge(0, &samples).temperature,
        Temperature::Shutdown
    );
}

#[test]
fn endstops_read_against_their_configuration() {
    let mut config = [EndstopConfig::UNUSED; ENDSTOPS];
    config[0] = EndstopConfig::SWITCH;
    config[1] = EndstopConfig::SWITCH;
    config[2] = EndstopConfig {
        active_low: false,
        ..EndstopConfig::SWITCH
    };
    // E0 high (clear), E1 low (triggered), E2 high with NC wiring (triggered),
    // E3 unused and low, E4/E5 unused and high.
    let inputs = 0b11_0101;
    assert_eq!(
        read_endstops(&config, inputs),
        [
            EndstopReading::Clear,
            EndstopReading::Triggered,
            EndstopReading::Triggered,
            EndstopReading::UnexpectedLow,
            EndstopReading::Empty,
            EndstopReading::Empty,
        ]
    );
}

fn report() -> BoardReport {
    BoardReport {
        rails: Rails { motor_mv: 24_000 },
        estop_closed: true,
        oled: None,
        endstop_config: [EndstopConfig::UNUSED; ENDSTOPS],
        endstops: [EndstopReading::Empty; ENDSTOPS],
        drivers: [
            DriverResult::Checked(DriverCheck::judge(0, &wired(0))),
            DriverResult::Checked(DriverCheck::judge(1, &wired(1))),
            DriverResult::Silent,
        ],
    }
}

#[test]
fn a_partly_populated_board_still_passes() {
    let report = report();
    assert!(report.passed());
    assert_eq!(report.warnings(), 0);
}

#[test]
fn faults_and_warnings_are_counted_and_named() {
    let mut report = report();
    report.estop_closed = false;
    report.endstops[3] = EndstopReading::UnexpectedLow;
    report.endstops[0] = EndstopReading::Triggered;
    let mut broken = wired(1);
    broken.high = reg::IOIN(broken.high.0 & !DIR);
    report.drivers[1] = DriverResult::Checked(DriverCheck::judge(1, &broken));
    report.drivers[2] = DriverResult::Mismatched(0x20);

    assert_eq!(report.failures(), 4);
    assert_eq!(report.warnings(), 1);
    let text = report.to_string();
    assert!(text.contains("DIR not seen by driver"), "{text}");
    assert!(text.contains("version 0x20"), "{text}");
    assert!(
        text.ends_with("result: 4 failure(s), 1 warning(s)"),
        "{text}"
    );
}

#[test]
fn a_driver_that_stops_answering_part_way_fails() {
    let mut report = report();
    report.drivers[2] = DriverResult::Intermittent;
    assert_eq!(report.failures(), 1);
}

#[test]
fn usb_power_leaves_drivers_untested_rather_than_failed() {
    let mut report = report();
    report.rails = Rails { motor_mv: 0 };
    report.drivers = [DriverResult::NoMotorSupply; 3];
    assert!(report.passed());
    assert_eq!(report.warnings(), 1);
}

#[test]
fn an_out_of_range_supply_fails() {
    let mut report = report();
    report.rails = Rails { motor_mv: 30_000 };
    assert!(!report.passed());
}
