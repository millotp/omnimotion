use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::time::Rate;
use esp_hal::uart::{Config as UartConfig, Uart};
use esp_hal::Async;
use omnimotion::devices::power::PowerSource;
use omnimotion::devices::tmc2209::{Bus, Driver, DriverConfig};
use omnimotion::{steps_per_mm, AxisConfig, EndstopConfig, AXES, ENDSTOPS};

use crate::diag::inventory::{self, DriverPresence, Inventory, Vmon};
use crate::motion::supervisor::FaultSupervisor;
use crate::motion::{stepper, Motion};
use crate::pins::{self, BoardPins};

pub type TmcBus = Bus<Uart<'static, Async>, embassy_time::Delay>;
pub type TmcDriver<'a> = Driver<'a, Uart<'static, Async>, embassy_time::Delay>;
pub type TmcError = <Uart<'static, Async> as embedded_io_async::ErrorType>::Error;

/// Everything `Board::start` needs. `Default` is a 200-step motor on 16
/// microsteps driving a 40 mm/rev belt, with no endstops fitted.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub axes: [AxisConfig; AXES],
    /// What is wired to J11-J16, in E0-E5 order.
    pub endstops: [EndstopConfig; ENDSTOPS],
    pub driver: DriverConfig,
    pub tick_hz: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            axes: [AxisConfig {
                steps_per_unit: steps_per_mm(200, 16, 40.0).expect("valid default calibration"),
                max_velocity: 60.0,
                max_acceleration: 600.0,
                min_velocity: 2.0,
            }; AXES],
            endstops: [EndstopConfig::UNUSED; ENDSTOPS],
            driver: DriverConfig::default(),
            tick_hz: stepper::DEFAULT_TICK_HZ,
        }
    }
}

#[derive(Debug)]
pub enum StartError {
    /// `enable` would reconfigure drivers that are already driving the motors.
    AlreadyEnabled,
    EStopOpen,
    PowerNotReady(PowerSource),
    /// No driver answered despite VMOTOR being inside the motor-ready window.
    NoDrivers,
    DriverMismatch {
        axis: usize,
        version: u8,
    },
    DriverConfig,
}

pub struct Board {
    pub motion: Motion,
    pub tmc: TmcBus,
    pub faults: FaultSupervisor,
    pub vmon: Vmon,
    /// Free for the application: the only thing on it is the OLED header.
    pub i2c: I2c<'static, Async>,
    pub inventory: Inventory,
}

impl Board {
    /// Surveys the board, configures every driver that answered and enables the
    /// outputs. Axes read zero where they stand.
    ///
    /// Call this from inside `boot_on_second_core`'s `init` to keep step
    /// generation off the core the application is using.
    ///
    /// The board is dropped on error and the step ISR cannot be installed twice,
    /// so an application that wants to retry, say once the e-stop closes or the
    /// bulk supply arrives, uses `new` and then `enable` instead.
    pub async fn start(pins: BoardPins, config: Config) -> Result<Self, StartError> {
        let mut board = Self::new(pins, &config).await;
        board.enable(&config).await?;
        Ok(board)
    }

    /// Checks the e-stop and supply, re-surveys and configures every driver
    /// that answers, then enables the outputs. Safe to call again after an
    /// error; refuses while the outputs are already enabled.
    pub async fn enable(&mut self, config: &Config) -> Result<(), StartError> {
        if self.faults.drivers_enabled() {
            return Err(StartError::AlreadyEnabled);
        }
        self.check_power()?;

        self.inventory.drivers = inventory::survey_drivers(&mut self.tmc).await;
        for (axis, driver) in self.inventory.drivers.iter().enumerate() {
            if let DriverPresence::Mismatched(version) = driver {
                return Err(StartError::DriverMismatch {
                    axis,
                    version: *version,
                });
            }
        }
        if self.inventory.drivers_responding() == 0 {
            return Err(StartError::NoDrivers);
        }

        for axis in 0..AXES {
            if self.inventory.drivers[axis] != DriverPresence::Responding {
                continue;
            }
            let mut driver = self.driver(axis).expect("axis in board range");
            if driver.apply(&config.driver).await.is_err() {
                return Err(StartError::DriverConfig);
            }
        }
        self.motion
            .set_available_axes(self.inventory.responding_axes());

        self.check_power()?;
        self.faults.enable_drivers();
        Ok(())
    }

    fn check_power(&mut self) -> Result<(), StartError> {
        self.inventory.rails = self.vmon.read();
        self.inventory.estop_closed = self.faults.estop_closed();
        if !self.inventory.estop_closed {
            return Err(StartError::EStopOpen);
        }
        if !self.inventory.can_run_motors() {
            return Err(StartError::PowerNotReady(self.inventory.source()));
        }
        Ok(())
    }

    /// Builds the board and surveys it, leaving the outputs disabled and the
    /// drivers unconfigured. `start` is the usual entry point.
    ///
    /// The step ISR is installed here, on the calling core.
    pub async fn new(peripherals: BoardPins, config: &Config) -> Self {
        // DIAG is push-pull, so only an unfitted channel relies on a pull. R62-R64
        // hold it low on the board; the internal pull covers boards built
        // without them.
        let pulled_down = InputConfig::default().with_pull(Pull::Down);
        let pulled_up = InputConfig::default().with_pull(Pull::Up);

        let mut i2c = I2c::new(
            peripherals.i2c0,
            I2cConfig::default().with_frequency(Rate::from_hz(pins::I2C_HZ)),
        )
        .expect("I2C0")
        .with_sda(peripherals.sda)
        .with_scl(peripherals.scl)
        .into_async();

        let uart = Uart::new(
            peripherals.uart1,
            UartConfig::default().with_baudrate(pins::TMC_BAUD),
        )
        .expect("UART1")
        .with_tx(peripherals.tx)
        .with_rx(peripherals.rx)
        .into_async();

        let oled = inventory::find_oled(&mut i2c).await;
        let mut vmon = Vmon::new(peripherals.adc1, peripherals.vmon);
        let rails = vmon.read();

        let estop = Input::new(peripherals.estop, pulled_up);
        let estop_closed = estop.is_low();
        let drv_en = Output::new(peripherals.drv_en, Level::Low, OutputConfig::default());

        let (stepper, watchdog) = stepper::start(
            peripherals.timg0,
            peripherals.step,
            peripherals.dir,
            config.tick_hz,
        );

        let mut tmc = Bus::new(uart, embassy_time::Delay);
        let drivers = inventory::survey_drivers(&mut tmc).await;
        let inventory = Inventory {
            oled,
            rails,
            estop_closed,
            drivers,
        };

        Self {
            motion: Motion::new(stepper, config.axes, inventory.responding_axes()),
            tmc,
            faults: FaultSupervisor::new(
                watchdog,
                estop,
                drv_en,
                peripherals.diag.map(|pin| Input::new(pin, pulled_down)),
                peripherals.endstops.map(|pin| Input::new(pin, pulled_up)),
                config.endstops,
            ),
            vmon,
            i2c,
            inventory,
        }
    }

    /// Re-asks each UART address what it is, and folds the answers into the
    /// inventory.
    pub async fn survey_drivers(&mut self) -> &[DriverPresence; AXES] {
        self.inventory.drivers = inventory::survey_drivers(&mut self.tmc).await;
        self.motion
            .retain_available_axes(self.inventory.responding_axes());
        &self.inventory.drivers
    }

    pub fn driver(&mut self, axis: usize) -> Option<TmcDriver<'_>> {
        omnimotion::devices::tmc2209::ADDRESSES
            .get(axis)
            .copied()
            .map(|address| Driver::new(&mut self.tmc, address))
    }
}
