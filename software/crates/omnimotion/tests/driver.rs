mod common;

use std::collections::{HashMap, VecDeque};

use common::Impatient;
use std::convert::Infallible;

use embedded_hal_async::delay::DelayNs;
use embedded_io::ErrorType;
use embedded_io_async::{Read, Write};
use omnimotion::devices::tmc2209::wire::{self, Pins, Request, GSTAT_FLAGS};
use omnimotion::devices::tmc2209::{
    current_scale, current_scales, microstep_resolution, tstep_for_rate, Bus, ConfigError, Driver,
    DriverConfig, Error, MicroStepResolution, R_SENSE, VERSION,
};
use tmc2209::reg;

/// A timeout that never fires, so every test but the timeout one exercises the
/// wire rather than the deadline.
struct Patient;

impl DelayNs for Patient {
    async fn delay_ns(&mut self, _ns: u32) {
        core::future::pending().await
    }
}

fn bus(address: u8) -> Bus<Wire, Patient> {
    Bus::new(Wire::new(address), Patient)
}

/// One TMC2209 sitting on the shared wire. Everything written is echoed back, as
/// it would be on real hardware, and read requests get a reply appended.
struct Wire {
    address: u8,
    registers: HashMap<u8, u32>,
    rx: VecDeque<u8>,
    corrupt_replies: usize,
    wrong_register_replies: usize,
    corrupt_echoes: usize,
    dropped_writes: usize,
    replies: usize,
    read_counts: HashMap<u8, usize>,
    write_order: Vec<u8>,
}

impl Wire {
    fn new(address: u8) -> Self {
        let mut registers = HashMap::new();
        registers.insert(
            reg::Address::IOIN as u8,
            wire::ioin(address, Pins::default()),
        );
        registers.insert(
            reg::Address::CHOPCONF as u8,
            wire::power_on(reg::Address::CHOPCONF),
        );
        registers.insert(reg::Address::IFCNT as u8, 0);
        Self {
            address,
            registers,
            rx: VecDeque::new(),
            corrupt_replies: 0,
            wrong_register_replies: 0,
            corrupt_echoes: 0,
            dropped_writes: 0,
            replies: 0,
            read_counts: HashMap::new(),
            write_order: Vec::new(),
        }
    }

    fn reply(&mut self, register: u8) {
        let data = self.registers.get(&register).copied().unwrap_or(0);
        self.replies += 1;
        *self.read_counts.entry(register).or_default() += 1;

        let mut frame = wire::reply(register, data);
        if self.corrupt_replies > 0 {
            self.corrupt_replies -= 1;
            frame[7] ^= 0xFF;
        }
        if self.wrong_register_replies > 0 {
            self.wrong_register_replies -= 1;
            frame[2] = frame[2].wrapping_add(1) & 0x7F;
            frame[7] = tmc2209::crc(&frame[..7]);
        }

        self.rx.extend(frame);
    }
}

impl ErrorType for Wire {
    type Error = Infallible;
}

impl Read for Wire {
    async fn read(&mut self, buffer: &mut [u8]) -> Result<usize, Infallible> {
        let count = buffer.len().min(self.rx.len());
        for slot in buffer.iter_mut().take(count) {
            *slot = self.rx.pop_front().unwrap();
        }
        Ok(count)
    }
}

impl Write for Wire {
    async fn write(&mut self, buffer: &[u8]) -> Result<usize, Infallible> {
        if self.corrupt_echoes > 0 {
            self.corrupt_echoes -= 1;
            let mut echo = buffer.to_vec();
            echo[0] ^= 1;
            self.rx.extend(echo);
        } else {
            self.rx.extend(buffer);
        }

        match wire::next_request(buffer).1 {
            Some(Request::Read { address, register }) if address == self.address => {
                self.reply(register);
            }
            Some(Request::Write {
                address,
                register,
                value,
            }) if address == self.address => {
                if self.dropped_writes > 0 {
                    self.dropped_writes -= 1;
                } else {
                    if register == reg::Address::GSTAT as u8 {
                        let previous = self.registers.get(&register).copied().unwrap_or(0);
                        self.registers
                            .insert(register, previous & !(value & GSTAT_FLAGS));
                    } else {
                        self.registers.insert(register, value);
                    }
                    self.write_order.push(register);
                    let ifcnt = self.registers[&(reg::Address::IFCNT as u8)] as u8;
                    self.registers
                        .insert(reg::Address::IFCNT as u8, ifcnt.wrapping_add(1) as u32);
                }
            }
            _ => {}
        }

        Ok(buffer.len())
    }

    async fn flush(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

/// A UART with nothing on the other end. Reads stay pending rather than
/// reporting EOF, which is how an absent driver really behaves.
struct Silent;

impl ErrorType for Silent {
    type Error = Infallible;
}

impl Read for Silent {
    async fn read(&mut self, _buffer: &mut [u8]) -> Result<usize, Infallible> {
        core::future::pending().await
    }
}

impl Write for Silent {
    async fn write(&mut self, buffer: &[u8]) -> Result<usize, Infallible> {
        Ok(buffer.len())
    }

    async fn flush(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

fn configured(config: &DriverConfig) -> HashMap<u8, u32> {
    let mut bus = bus(0);
    pollster::block_on(async {
        Driver::new(&mut bus, 0).apply(config).await.unwrap();
    });
    bus.into_inner().registers
}

#[test]
fn full_scale_current_matches_the_board() {
    // The public limit is intentionally below this fitted-shunt full scale.
    let full_scale = tmc2209::vsense_cs_to_rms_current(R_SENSE, false, 31);
    assert!(
        (full_scale - 1768.0).abs() < 5.0,
        "full scale was {full_scale} mA"
    );

    assert_eq!(current_scale(full_scale, false), 31);
}

#[test]
fn hold_current_is_scaled_against_the_run_current_vsense() {
    let (vsense, run, hold) = current_scales(800.0, 400.0);

    let run_ma = tmc2209::vsense_cs_to_rms_current(R_SENSE, vsense, run);
    let hold_ma = tmc2209::vsense_cs_to_rms_current(R_SENSE, vsense, hold);

    assert!((run_ma - 800.0).abs() < 40.0, "run was {run_ma} mA");
    assert!((hold_ma - 400.0).abs() < 40.0, "hold was {hold_ma} mA");
}

#[test]
fn setup_takes_the_driver_off_its_reset_defaults() {
    let registers = configured(&DriverConfig::default());
    let gconf = reg::GCONF(registers[&(reg::Address::GCONF as u8)]);

    // MS1/MS2 are address straps on this board, so microstepping has to come
    // from CHOPCONF and the VREF divider must not own the current.
    assert!(gconf.mstep_reg_select());
    assert!(!gconf.i_scale_analog());
    assert!(gconf.pdn_disable());
    assert!(
        !gconf.en_spread_cycle(),
        "default config asks for StealthChop"
    );
}

#[test]
fn setup_clears_sticky_global_status_flags() {
    let mut wire = Wire::new(0);
    wire.registers.insert(reg::Address::GSTAT as u8, 0b010);
    let mut bus = Bus::new(wire, Patient);

    pollster::block_on(async {
        Driver::new(&mut bus, 0)
            .apply(&DriverConfig::default())
            .await
            .unwrap();
    });

    let wire = bus.into_inner();
    assert_eq!(wire.registers[&(reg::Address::GSTAT as u8)], 0);
    assert!(wire.write_order.contains(&(reg::Address::GSTAT as u8)));
}

#[test]
fn take_reset_reports_a_reset_once() {
    let mut wire = Wire::new(0);
    wire.registers.insert(reg::Address::GSTAT as u8, 0b001);
    let mut bus = Bus::new(wire, Patient);

    let (first, second) = pollster::block_on(async {
        let mut driver = Driver::new(&mut bus, 0);
        (
            driver.take_reset().await.unwrap(),
            driver.take_reset().await.unwrap(),
        )
    });

    assert!(first);
    assert!(!second);
}

#[test]
fn setup_rejects_an_active_charge_pump_fault() {
    let mut wire = Wire::new(0);
    wire.registers.insert(reg::Address::GSTAT as u8, 0b100);
    let mut bus = Bus::new(wire, Patient);

    let result = pollster::block_on(async {
        Driver::new(&mut bus, 0)
            .apply(&DriverConfig::default())
            .await
    });

    assert!(matches!(result, Err(Error::DriverFault)));
}

#[test]
fn microstepping_and_current_reach_the_chopper() {
    let config = DriverConfig {
        microsteps: MicroStepResolution::from_microsteps(32),
        run_current_ma: 1_200.0,
        ..DriverConfig::default()
    };
    let registers = configured(&config);

    let chopconf = reg::CHOPCONF(registers[&(reg::Address::CHOPCONF as u8)]);
    assert_eq!(chopconf.mres().number_of_microsteps(), 32);
    assert!(chopconf.intpol());

    let current = reg::IHOLD_IRUN(registers[&(reg::Address::IHOLD_IRUN as u8)]);
    let (vsense, run, hold) = current_scales(1_200.0, 400.0);
    assert_eq!(current.irun(), run);
    assert_eq!(current.ihold(), hold);
    assert_eq!(chopconf.vsense(), vsense);
}

#[test]
fn spreadcycle_survives_a_read_modify_write() {
    let mut bus = bus(0);

    pollster::block_on(async {
        let mut driver = Driver::new(&mut bus, 0);
        driver.apply(&DriverConfig::default()).await.unwrap();
        driver.set_spreadcycle(true).await.unwrap();
    });

    let registers = bus.into_inner().registers;
    let gconf = reg::GCONF(registers[&(reg::Address::GCONF as u8)]);

    assert!(gconf.en_spread_cycle());
    assert!(gconf.mstep_reg_select(), "the rest of GCONF was clobbered");
}

#[test]
fn a_read_survives_its_own_echo() {
    let mut bus = bus(1);
    let version = pollster::block_on(async { Driver::new(&mut bus, 1).version().await.unwrap() });

    assert_eq!(version, VERSION);
}

#[test]
fn a_corrupt_reply_is_retried() {
    let mut wire = Wire::new(0);
    wire.corrupt_replies = 2;
    let mut bus = Bus::new(wire, Patient);

    let version = pollster::block_on(async { Driver::new(&mut bus, 0).version().await.unwrap() });
    assert_eq!(version, VERSION);
}

#[test]
fn a_driver_that_never_answers_is_reported() {
    let mut wire = Wire::new(0);
    wire.corrupt_replies = usize::MAX;
    let mut bus = Bus::new(wire, Patient);

    let result = pollster::block_on(async { Driver::new(&mut bus, 0).version().await });
    assert!(matches!(result, Err(Error::NoReply)), "expected NoReply");
}

#[test]
fn an_absent_driver_is_reported() {
    let mut bus = bus(0);
    let result = pollster::block_on(async { Driver::new(&mut bus, 2).version().await });

    assert!(matches!(result, Err(Error::NoReply)), "expected NoReply");
}

#[test]
fn a_silent_wire_times_out_instead_of_hanging() {
    // Run it off the test thread: if the timeout ever regresses this future
    // never completes, and a hang is much harder to diagnose than a failure.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bus = Bus::new(Silent, Impatient);
        let result = pollster::block_on(async { Driver::new(&mut bus, 0).version().await });
        let _ = tx.send(matches!(result, Err(Error::NoReply)));
    });

    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(reported) => assert!(reported, "expected NoReply"),
        Err(_) => panic!("the bus hung on a driver that never replies"),
    }
}

#[test]
fn set_current_updates_chopconf_without_reading_it_back() {
    let mut bus = bus(0);
    let config = DriverConfig {
        microsteps: MicroStepResolution::from_microsteps(32),
        ..DriverConfig::default()
    };

    pollster::block_on(async {
        let mut driver = Driver::new(&mut bus, 0);
        driver.apply(&config).await.unwrap();
        driver.set_current(1_300.0, 650.0).await.unwrap();
    });

    let wire = bus.into_inner();
    assert_eq!(
        wire.read_counts
            .get(&(reg::Address::CHOPCONF as u8))
            .copied()
            .unwrap_or(0),
        0,
        "set_current read CHOPCONF back despite its confirmed shadow"
    );

    let (vsense, run, hold) = current_scales(1_300.0, 650.0);
    let chopconf = reg::CHOPCONF(wire.registers[&(reg::Address::CHOPCONF as u8)]);
    assert_eq!(chopconf.vsense(), vsense);
    assert_eq!(
        chopconf.mres().number_of_microsteps(),
        32,
        "the shadow lost the microstep setting"
    );

    let current = reg::IHOLD_IRUN(wire.registers[&(reg::Address::IHOLD_IRUN as u8)]);
    assert_eq!(current.irun(), run);
    assert_eq!(current.ihold(), hold);
}

#[test]
fn a_driver_with_no_shadow_yet_reads_chopconf_first() {
    let mut bus = bus(0);

    pollster::block_on(async {
        Driver::new(&mut bus, 0)
            .set_microsteps(MicroStepResolution::from_microsteps(64))
            .await
            .unwrap();
    });

    let wire = bus.into_inner();
    assert_eq!(
        wire.read_counts[&(reg::Address::CHOPCONF as u8)],
        1,
        "expected exactly one CHOPCONF read-back"
    );

    let chopconf = reg::CHOPCONF(wire.registers[&(reg::Address::CHOPCONF as u8)]);
    assert_eq!(chopconf.mres().number_of_microsteps(), 64);
    // The wire powers up with TOFF=3, which the read-modify-write must keep.
    assert_eq!(chopconf.toff(), 3);
}

#[test]
fn a_direct_chopconf_write_invalidates_the_shared_shadow() {
    let mut bus = bus(0);

    pollster::block_on(async {
        Driver::new(&mut bus, 0)
            .apply(&DriverConfig::default())
            .await
            .unwrap();

        let mut direct = reg::CHOPCONF::default();
        direct.set_mres(MicroStepResolution::from_microsteps(64));
        bus.write(0, direct).await.unwrap();

        Driver::new(&mut bus, 0)
            .set_current(900.0, 450.0)
            .await
            .unwrap();
    });

    let wire = bus.into_inner();
    assert_eq!(
        wire.read_counts[&(reg::Address::CHOPCONF as u8)],
        1,
        "the stale shared shadow was used after a direct write"
    );
    let chopconf = reg::CHOPCONF(wire.registers[&(reg::Address::CHOPCONF as u8)]);
    assert_eq!(chopconf.mres().number_of_microsteps(), 64);
}

#[test]
fn tstep_tracks_the_step_rate() {
    // 1000 steps/s at 1/16 is 16 000 microsteps/s at 1/256 resolution, and the
    // driver clock is 12 MHz.
    assert_eq!(tstep_for_rate(1_000.0, 16), Ok(750));
    assert_eq!(tstep_for_rate(0.0, 16), Err(ConfigError::InvalidRate));
}

#[test]
fn writes_retry_until_ifcnt_confirms_them() {
    let mut wire = Wire::new(0);
    wire.dropped_writes = 1;
    let mut bus = Bus::new(wire, Patient);

    pollster::block_on(async {
        let mut gconf = reg::GCONF(0);
        gconf.set_pdn_disable(true);
        bus.write(0, gconf).await.unwrap();
    });

    let wire = bus.into_inner();
    assert_eq!(wire.write_order, vec![reg::Address::GCONF as u8]);
    assert_eq!(wire.registers[&(reg::Address::IFCNT as u8)], 1);
}

#[test]
fn a_write_never_confirmed_by_ifcnt_is_reported() {
    let mut wire = Wire::new(0);
    wire.dropped_writes = usize::MAX;
    let mut bus = Bus::new(wire, Patient);

    let result = pollster::block_on(async { bus.write(0, reg::GCONF(0)).await });

    assert!(matches!(result, Err(Error::WriteNotConfirmed)));
    assert!(bus.into_inner().write_order.is_empty());
}

#[test]
fn ifcnt_confirmation_is_wrap_aware() {
    let mut wire = Wire::new(0);
    wire.registers.insert(reg::Address::IFCNT as u8, 255);
    let mut bus = Bus::new(wire, Patient);

    pollster::block_on(async {
        bus.write(0, reg::GCONF(0)).await.unwrap();
    });

    assert_eq!(bus.into_inner().registers[&(reg::Address::IFCNT as u8)], 0);
}

#[test]
fn wrong_register_and_bad_echo_are_retried() {
    let mut wire = Wire::new(0);
    wire.wrong_register_replies = 1;
    wire.corrupt_echoes = 1;
    let mut bus = Bus::new(wire, Patient);

    let version = pollster::block_on(async { Driver::new(&mut bus, 0).version().await.unwrap() });
    assert_eq!(version, VERSION);
}

#[test]
fn repeated_wrong_register_is_reported_after_retries() {
    let mut wire = Wire::new(0);
    wire.wrong_register_replies = usize::MAX;
    let mut bus = Bus::new(wire, Patient);

    let result = pollster::block_on(async { Driver::new(&mut bus, 0).version().await });
    assert!(matches!(result, Err(Error::WrongRegister)));
}

#[test]
fn invalid_address_and_configuration_are_rejected_without_writes() {
    let mut bus = bus(0);
    let invalid = DriverConfig {
        run_current_ma: 1_500.0,
        ..DriverConfig::default()
    };

    pollster::block_on(async {
        assert!(matches!(
            Driver::new(&mut bus, 4).version().await,
            Err(Error::InvalidAddress(4))
        ));
        assert!(matches!(
            Driver::new(&mut bus, 0).apply(&invalid).await,
            Err(Error::InvalidConfiguration(ConfigError::InvalidCurrent))
        ));
    });
    assert!(bus.into_inner().write_order.is_empty());
}

#[test]
fn validated_microsteps_and_stallguard_inputs_reject_bad_values() {
    assert!(matches!(
        microstep_resolution(3),
        Err(ConfigError::InvalidMicrosteps)
    ));
    assert_eq!(
        tstep_for_rate(1_000.0, 3),
        Err(ConfigError::InvalidMicrosteps)
    );
    assert_eq!(tstep_for_rate(f32::NAN, 16), Err(ConfigError::InvalidRate));
}

#[test]
fn digital_current_control_is_enabled_after_current_is_bounded() {
    let mut bus = bus(0);
    pollster::block_on(async {
        Driver::new(&mut bus, 0)
            .apply(&DriverConfig::default())
            .await
            .unwrap();
    });

    let order = bus.into_inner().write_order;
    let currents = order
        .iter()
        .position(|register| *register == reg::Address::IHOLD_IRUN as u8)
        .unwrap();
    let gconf = order
        .iter()
        .position(|register| *register == reg::Address::GCONF as u8)
        .unwrap();
    assert!(currents < gconf);
}
