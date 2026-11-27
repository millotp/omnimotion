use omnimotion::devices::tmc2209::{DriverProblem, DriverStatus};
use tmc2209::reg;

const OTPW: u32 = 1 << 0;
const OT: u32 = 1 << 1;
const S2GA: u32 = 1 << 2;
const OLA: u32 = 1 << 6;

fn status(bits: u32) -> DriverStatus {
    DriverStatus::from_registers(0x21, reg::DRV_STATUS(bits))
}

#[test]
fn a_quiet_driver_has_no_problem() {
    assert_eq!(status(0).problem(), None);
    assert!(status(0).connected);
}

#[test]
fn an_open_load_is_not_a_problem() {
    assert!(status(OLA).open_load_a);
    assert_eq!(status(OLA).problem(), None);
}

#[test]
fn the_worst_report_wins() {
    assert_eq!(DriverStatus::UNKNOWN.problem(), Some(DriverProblem::Silent));
    assert_eq!(
        status(OT | OTPW | S2GA).problem(),
        Some(DriverProblem::Overtemperature)
    );
    assert_eq!(
        status(OTPW | S2GA).problem(),
        Some(DriverProblem::OvertemperatureWarning)
    );
    assert_eq!(status(S2GA).problem(), Some(DriverProblem::ShortCircuit));
}
