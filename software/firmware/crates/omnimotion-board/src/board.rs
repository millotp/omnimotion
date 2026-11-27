use embassy_time::{Duration, Timer};
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::time::Rate;
#[cfg(not(feature = "virtual-hardware"))]
use esp_hal::uart::{Config as UartConfig, Uart};
use esp_hal::Async;
use omnimotion::devices::power::PowerSource;
use omnimotion::devices::tmc2209::{Bus, Driver, DriverConfig};
use omnimotion::{steps_per_mm, AxisConfig, AxisMask, EndstopConfig, AXES, ENDSTOPS};

use crate::diag::inventory::{self, DriverPresence, Inventory, Vmon};
use crate::motion::supervisor::{self, FaultSupervisor};
use crate::motion::{stepper, Motion};
use crate::pins::{self, BoardPins};

#[cfg(not(feature = "virtual-hardware"))]
type TmcSerial = Uart<'static, Async>;
/// With nothing attached, the drivers are an emulator behind the same bus.
#[cfg(feature = "virtual-hardware")]
type TmcSerial = &'static mut omnimotion::devices::tmc2209::emulator::TmcEmulator;

pub type TmcBus = Bus<TmcSerial, embassy_time::Delay>;
pub type TmcDriver<'a> = Driver<'a, TmcSerial, embassy_time::Delay>;
pub type TmcError = <TmcSerial as embedded_io_async::ErrorType>::Error;

/// How often `configure_when_ready` tries again.
const READY_RETRY: Duration = Duration::from_secs(1);

/// Everything `Board::start` needs. `Default` is a 200-step motor on 16
/// microsteps driving a 40 mm/rev belt, with no endstops fitted.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub axes: [AxisConfig; AXES],
    /// What is wired to J11-J16, in E0-E5 order.
    pub endstops: [EndstopConfig; ENDSTOPS],
    /// Applied to each axis's driver every time the outputs come on.
    pub drivers: [DriverConfig; AXES],
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
            drivers: [DriverConfig::default(); AXES],
        }
    }
}

#[derive(Debug)]
pub enum StartError {
    /// `enable` would reconfigure drivers that are already driving the motors.
    AlreadyEnabled,
    /// A fault is latched; `faults.clear` first.
    FaultLatched,
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

impl StartError {
    /// Whether waiting can cure it: the e-stop closing or the supply arriving.
    pub fn is_waiting(&self) -> bool {
        matches!(self, Self::EStopOpen | Self::PowerNotReady(_))
    }
}

impl core::fmt::Display for StartError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AlreadyEnabled => f.write_str("the motor drivers are already enabled"),
            Self::FaultLatched => f.write_str("a fault is latched"),
            Self::EStopOpen => f.write_str("e-stop open"),
            Self::PowerNotReady(PowerSource::UsbOnly) => {
                f.write_str("no motor supply, the board runs on USB alone")
            }
            Self::PowerNotReady(PowerSource::BulkOutOfRange) => {
                f.write_str("motor supply voltage out of range")
            }
            Self::PowerNotReady(PowerSource::Bulk) => f.write_str("motor supply not ready"),
            Self::NoDrivers => f.write_str("no motor driver answered"),
            Self::DriverMismatch { axis, version } => write!(
                f,
                "the driver on M{axis} is not a TMC2209 (version {version:#04x})"
            ),
            Self::DriverConfig => f.write_str("a motor driver rejected its configuration"),
        }
    }
}

pub struct Board {
    pub motion: Motion,
    pub tmc: TmcBus,
    pub faults: FaultSupervisor,
    /// Free for the application: the only thing on it is the OLED header.
    pub i2c: I2c<'static, Async>,
    pub inventory: Inventory,
    drivers: [DriverConfig; AXES],
}

impl Board {
    /// Surveys the board, configures every driver that answered and enables the
    /// outputs. Axes read zero where they stand.
    ///
    /// Call this from inside `boot_on_second_core`'s `init` to keep step
    /// generation off the core the application is using.
    ///
    /// The board is dropped on error and the step ISR cannot be installed twice,
    /// so an application that wants to wait, say for the e-stop to close or the
    /// bulk supply to arrive, uses `new` and then `configure_when_ready`.
    pub async fn start(pins: BoardPins, config: Config) -> Result<Self, StartError> {
        let mut board = Self::new(pins, &config).await;
        board.enable().await?;
        Ok(board)
    }

    /// Checks the e-stop and supply, configures again every driver the survey
    /// found (`configure` surveys first if none has answered yet), then
    /// enables the outputs. Returns the axes whose driver had reset since it
    /// was last configured (a supply drop does that, even with the outputs
    /// off), which may have jumped up to two full steps. Safe to call again
    /// after an error; refuses while the outputs are already enabled.
    pub async fn enable(&mut self) -> Result<AxisMask, StartError> {
        let reset = if self.inventory.drivers_responding() == 0 {
            self.configure().await?
        } else {
            self.ready_to_configure()?;
            let reset = self.apply_drivers().await?;
            self.check_power()?;
            reset
        };
        self.faults.enable_drivers();
        Ok(reset)
    }

    /// Surveys the drivers, configures every one that answers and sets the
    /// available axes, but leaves the outputs off, so nothing moves or holds.
    /// `enable` turns them on later.
    pub async fn configure(&mut self) -> Result<AxisMask, StartError> {
        self.ready_to_configure()?;
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
        let reset = self.apply_drivers().await?;
        self.motion
            .set_available_axes(self.inventory.responding_axes());

        self.check_power()?;
        Ok(reset)
    }

    fn ready_to_configure(&mut self) -> Result<(), StartError> {
        if self.faults.drivers_enabled() {
            return Err(StartError::AlreadyEnabled);
        }
        if supervisor::latched() != 0 {
            return Err(StartError::FaultLatched);
        }
        self.check_power()
    }

    /// Writes each surveyed driver's configuration, returning the axes whose
    /// driver had reset.
    async fn apply_drivers(&mut self) -> Result<AxisMask, StartError> {
        let mut reset = 0;
        for axis in 0..AXES {
            if self.inventory.drivers[axis] != DriverPresence::Responding {
                continue;
            }
            let config = self.drivers[axis];
            let mut driver = self.driver(axis).expect("axis in board range");
            if driver.take_reset().await.unwrap_or(true) {
                reset |= 1 << axis;
            }
            if driver.apply(&config).await.is_err() {
                return Err(StartError::DriverConfig);
            }
        }
        Ok(reset)
    }

    /// `configure`, tried again every second while the e-stop is open or the
    /// motor supply is missing; `waiting` hears each such refusal. Any other
    /// error ends it.
    pub async fn configure_when_ready(
        &mut self,
        mut waiting: impl FnMut(&StartError),
    ) -> Result<AxisMask, StartError> {
        loop {
            match self.configure().await {
                Err(error) if error.is_waiting() => waiting(&error),
                result => return result,
            }
            Timer::after(READY_RETRY).await;
        }
    }

    fn check_power(&mut self) -> Result<(), StartError> {
        self.inventory.rails = self.faults.rails();
        self.inventory.estop_closed = self.faults.estop_closed();
        if !self.inventory.estop_closed {
            return Err(StartError::EStopOpen);
        }
        if !self.inventory.can_run_motors() {
            return Err(StartError::PowerNotReady(self.inventory.source()));
        }
        Ok(())
    }

    /// The configuration `enable` applies to `axis`'s driver.
    pub fn driver_config(&self, axis: usize) -> Option<DriverConfig> {
        self.drivers.get(axis).copied()
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

        #[cfg(not(feature = "virtual-hardware"))]
        let uart = Uart::new(
            peripherals.uart1,
            UartConfig::default().with_baudrate(pins::TMC_BAUD),
        )
        .expect("UART1")
        .with_tx(peripherals.tx)
        .with_rx(peripherals.rx)
        .into_async();
        // M0 and M1 are fitted; the third socket is empty, as on the machine.
        #[cfg(feature = "virtual-hardware")]
        // In a static so the board moves a pointer, not the emulator's registers, through frames of the
        // application core's small stack.
        let uart = {
            static EMULATOR: static_cell::StaticCell<
                omnimotion::devices::tmc2209::emulator::TmcEmulator,
            > = static_cell::StaticCell::new();
            EMULATOR.init_with(|| {
                omnimotion::devices::tmc2209::emulator::TmcEmulator::new([true, true, false])
                    .with_pins(crate::virtual_hardware::pins)
            })
        };

        let oled = inventory::find_oled(&mut i2c).await;
        let mut vmon = Vmon::new(peripherals.adc1, peripherals.vmon);
        let rails = vmon.read();

        let estop = Input::new(peripherals.estop, pulled_up);
        let estop_closed = estop.is_low();
        let drv_en = Output::new(peripherals.drv_en, Level::Low, OutputConfig::default());

        let (stepper, watchdog) =
            stepper::start(peripherals.timg0, peripherals.step, peripherals.dir);

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
                vmon,
            ),
            i2c,
            inventory,
            drivers: config.drivers,
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
