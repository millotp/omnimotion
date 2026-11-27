mod common;

use core::sync::atomic::{AtomicBool, Ordering};

use common::Impatient;
use omnimotion::devices::check::{DriverCheck, DriverSamples, Temperature};
use omnimotion::devices::tmc2209::emulator::{StepDir, TmcEmulator};
use omnimotion::devices::tmc2209::{Bus, Driver, DriverConfig, Error, ADDRESSES, VERSION};

fn bus() -> Bus<TmcEmulator, Impatient> {
    Bus::new(TmcEmulator::new([true, true, false]), Impatient)
}

#[test]
fn fitted_drivers_report_a_tmc2209() {
    let mut bus = bus();
    pollster::block_on(async {
        for address in [ADDRESSES[0], ADDRESSES[1]] {
            assert_eq!(
                Driver::new(&mut bus, address).version().await.unwrap(),
                VERSION
            );
        }
    });
}

#[test]
fn an_empty_socket_stays_silent() {
    let mut bus = bus();
    pollster::block_on(async {
        let result = Driver::new(&mut bus, ADDRESSES[2]).version().await;
        assert!(matches!(result, Err(Error::NoReply)), "{result:?}");
    });
}

#[test]
fn a_driver_takes_its_configuration_and_reports_healthy() {
    let mut bus = bus();
    pollster::block_on(async {
        for address in [ADDRESSES[0], ADDRESSES[1]] {
            let mut driver = Driver::new(&mut bus, address);
            driver.apply(&DriverConfig::default()).await.unwrap();
            let status = driver.status().await.unwrap();
            assert!(status.stst(), "the motor stands still");
            assert!(!status.ot() && !status.otpw());
            assert!(!status.s2ga() && !status.s2gb() && !status.s2vsa() && !status.s2vsb());
            assert!(!status.ola() && !status.olb());
        }
    });
}

#[test]
fn a_reset_is_reported_once() {
    let mut bus = bus();
    pollster::block_on(async {
        let mut driver = Driver::new(&mut bus, ADDRESSES[0]);
        assert!(driver.take_reset().await.unwrap());
        assert!(!driver.take_reset().await.unwrap());
    });
}

#[test]
fn drivers_keep_separate_registers() {
    let mut bus = bus();
    pollster::block_on(async {
        let mut first = Driver::new(&mut bus, ADDRESSES[0]);
        first.take_reset().await.unwrap();
        let mut second = Driver::new(&mut bus, ADDRESSES[1]);
        assert!(second.take_reset().await.unwrap());
    });
}

static LINES_HIGH: AtomicBool = AtomicBool::new(false);

fn lines(_address: usize) -> StepDir {
    let high = LINES_HIGH.load(Ordering::Relaxed);
    StepDir {
        step: high,
        dir: high,
    }
}

#[test]
fn the_wiring_check_sees_the_lines_it_drives() {
    let mut bus = Bus::new(
        TmcEmulator::new([true, true, false]).with_pins(lines),
        Impatient,
    );
    pollster::block_on(async {
        for address in [ADDRESSES[0], ADDRESSES[1]] {
            let mut driver = Driver::new(&mut bus, address);
            LINES_HIGH.store(false, Ordering::Relaxed);
            let low = driver.inputs().await.unwrap();
            LINES_HIGH.store(true, Ordering::Relaxed);
            let high = driver.inputs().await.unwrap();
            LINES_HIGH.store(false, Ordering::Relaxed);
            let global = driver.global_status().await.unwrap();
            let status = driver.status().await.unwrap();

            let check = DriverCheck::judge(
                address,
                &DriverSamples {
                    low,
                    high,
                    mcu_diag: [false, false],
                    global,
                    status,
                },
            );
            assert!(check.passed(), "driver {address}: {check:?}");
            assert_eq!(check.temperature, Temperature::Normal);
        }
    });
}
