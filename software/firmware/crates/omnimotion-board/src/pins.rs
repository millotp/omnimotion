//! Pin map, assigned in `crate::split`.
//!
//! GPIO33-37 never appear: 35/36/37 are the N16R8's octal PSRAM and 33/34 are
//! not bonded out on the WROOM-1. GPIO0 is the boot strap, GPIO19/20 are USB
//! and GPIO43/44 are the debug UART, so none of them are available either.
//! GPIO45 and GPIO46 are the only spare pins left.
//!
//! | Signal        | GPIO           |
//! |---------------|----------------|
//! | STEP 0-2      | 4, 5, 6        |
//! | DIR 0-2       | 7, 8, 9        |
//! | DIAG 0-2      | 10, 11, 12 (R62-R64 pull-downs) |
//! | E0-E5         | 38, 39, 40, 41, 13, 14 |
//! | ESTOP_SENSE   | 15             |
//! | DRV_EN        | 21             |
//! | TMC UART TX/RX| 16, 47         |
//! | I2C SDA/SCL   | 17, 18         |
//! | VMON          | 1 (ADC1_CH0)   |
//! | OUT1_EN       | 42             |
//! | OUT2_EN       | 2              |
//! | AIN           | 3 (ADC1_CH2)   |
//! | RGB_DATA      | 48             |

use esp_hal::gpio::AnyPin;
use esp_hal::peripherals::{ADC1, GPIO1, I2C0, TIMG0, UART1};
use omnimotion::{AXES, ENDSTOPS};

/// SSD1306 modules strap either address, so the survey tries both.
pub const OLED_ADDRESSES: [u8; 2] = [0x3C, 0x3D];

pub const I2C_HZ: u32 = 400_000;

/// The TMC2209 allows 500 kbit/s; 115200 leaves margin on a bus shared by three.
pub const TMC_BAUD: u32 = 115_200;

/// Everything `Board::new` claims, bundled so it can be moved onto whichever
/// core runs the board. Built by [`crate::split!`], which ties every field to the copper.
pub struct BoardPins {
    #[doc(hidden)]
    pub i2c0: I2C0<'static>,
    #[doc(hidden)]
    pub sda: AnyPin<'static>,
    #[doc(hidden)]
    pub scl: AnyPin<'static>,
    #[doc(hidden)]
    pub uart1: UART1<'static>,
    #[doc(hidden)]
    pub tx: AnyPin<'static>,
    #[doc(hidden)]
    pub rx: AnyPin<'static>,
    #[doc(hidden)]
    pub adc1: ADC1<'static>,
    #[doc(hidden)]
    pub vmon: GPIO1<'static>,
    #[doc(hidden)]
    pub timg0: TIMG0<'static>,
    #[doc(hidden)]
    pub step: [AnyPin<'static>; AXES],
    #[doc(hidden)]
    pub dir: [AnyPin<'static>; AXES],
    #[doc(hidden)]
    pub diag: [AnyPin<'static>; AXES],
    #[doc(hidden)]
    pub endstops: [AnyPin<'static>; ENDSTOPS],
    #[doc(hidden)]
    pub estop: AnyPin<'static>,
    #[doc(hidden)]
    pub drv_en: AnyPin<'static>,
}

/// Moves the scheduler's and the board's peripherals out of a local
/// `Peripherals` binding, returning `(Rtos, BoardPins)`. Everything else stays
/// in the binding for the application: FLASH, WIFI, the OUT1/OUT2 gates on
/// GPIO42/GPIO2, AIN on GPIO3 and RGB_DATA on GPIO48. It is a macro because
/// only a local binding can be partially moved out of.
///
/// Splitting here rather than inside `Board::new` is what lets the board be
/// built on a core other than the one that ran `esp_hal::init`.
#[macro_export]
macro_rules! split {
    ($peripherals:ident) => {
        (
            $crate::Rtos {
                systimer: $peripherals.SYSTIMER,
                software_interrupt: $peripherals.SW_INTERRUPT,
                cpu_control: $peripherals.CPU_CTRL,
            },
            $crate::BoardPins {
                i2c0: $peripherals.I2C0,
                sda: $peripherals.GPIO17.into(),
                scl: $peripherals.GPIO18.into(),
                uart1: $peripherals.UART1,
                tx: $peripherals.GPIO16.into(),
                rx: $peripherals.GPIO47.into(),
                adc1: $peripherals.ADC1,
                vmon: $peripherals.GPIO1,
                timg0: $peripherals.TIMG0,
                step: [
                    $peripherals.GPIO4.into(),
                    $peripherals.GPIO5.into(),
                    $peripherals.GPIO6.into(),
                ],
                dir: [
                    $peripherals.GPIO7.into(),
                    $peripherals.GPIO8.into(),
                    $peripherals.GPIO9.into(),
                ],
                diag: [
                    $peripherals.GPIO10.into(),
                    $peripherals.GPIO11.into(),
                    $peripherals.GPIO12.into(),
                ],
                endstops: [
                    $peripherals.GPIO38.into(),
                    $peripherals.GPIO39.into(),
                    $peripherals.GPIO40.into(),
                    $peripherals.GPIO41.into(),
                    $peripherals.GPIO13.into(),
                    $peripherals.GPIO14.into(),
                ],
                estop: $peripherals.GPIO15.into(),
                drv_en: $peripherals.GPIO21.into(),
            },
        )
    };
}
