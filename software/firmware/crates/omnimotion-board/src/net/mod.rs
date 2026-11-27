//! WiFi station, DHCP, and the firmware update endpoint.
//!
//! The credentials and the update token are compile-time environment
//! variables, so the image flashed over USB joins the network on its own and
//! every later image can arrive over WiFi. An image that cannot get an address
//! within its trial gives its slot back, so a bad update costs a reboot rather
//! than a USB cable.

pub mod http;
mod ota;

use core::fmt::Write as _;

use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_net::tcp::TcpSocket;
use embassy_net::{DhcpConfig, Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::delay::Delay;
use esp_hal::peripherals::WIFI;
use esp_hal::rng::Rng;
use esp_hal::system::software_reset;
use esp_radio::wifi::sta::StationConfig;
use esp_radio::wifi::{Config as WifiConfig, ControllerConfig, Interface, WifiController};
use heapless::String;
use omnimotion::update::ota::{parse_upload, Upload};
use static_cell::StaticCell;

use crate::net::ota::{Boot, Ota, OtaError};
use crate::storage::flash::Lease;

const SSID: &str = match option_env!("OMNIMOTION_WIFI_SSID") {
    Some(value) => value,
    None => "",
};
const PASSWORD: &str = match option_env!("OMNIMOTION_WIFI_PASSWORD") {
    Some(value) => value,
    None => "",
};
const TOKEN: &str = match option_env!("OMNIMOTION_OTA_TOKEN") {
    Some(value) => value,
    None => "",
};

/// `POST /ota` with the image as the body and the token in `X-API-Key`.
pub const OTA_PORT: u16 = 8080;
/// One for DHCP and one for updates; the rest are the application's.
pub const SOCKETS: usize = 6;

/// How long a freshly installed image has to get an address before it gives
/// its slot back.
const TRIAL: Duration = Duration::from_secs(120);
const RETRY: Duration = Duration::from_secs(1);
const HEAD_LIMIT: usize = 1_024;

/// Joins the network and serves updates from the calling core's executor.
/// Returns the stack so the application can open its own sockets on it.
///
/// Call it as soon as that executor runs: a new image that reset during its
/// trial is rejected here. esp-radio needs the scheduler, so after `boot` or
/// `boot_on_second_core`, never before.
pub fn start(spawner: Spawner, wifi: WIFI<'static>) -> Stack<'static> {
    static OTA: StaticCell<Ota> = StaticCell::new();
    let ota = OTA.init(Ota::new());
    let configured = !SSID.is_empty() && !TOKEN.is_empty();

    let lease = loop {
        match Lease::try_brief() {
            Some(lease) => break lease,
            None => Delay::new().delay_millis(1),
        }
    };
    let mut boot = ota
        .start_trial(&lease)
        .expect("the partition table has otadata and two OTA slots");
    if boot == Boot::Trial && !configured {
        let _ = ota.reject(&lease);
        boot = Boot::Failed;
    }
    drop(lease);
    if boot == Boot::Failed {
        software_reset();
    }
    assert!(
        configured,
        "build with OMNIMOTION_WIFI_SSID, OMNIMOTION_WIFI_PASSWORD and OMNIMOTION_OTA_TOKEN set"
    );

    let station = WifiConfig::Station(
        StationConfig::default()
            .with_ssid(SSID)
            .with_password(PASSWORD.into()),
    );
    let (controller, interfaces) = esp_radio::wifi::new(
        wifi,
        ControllerConfig::default().with_initial_config(station),
    )
    .expect("WiFi controller");

    static RESOURCES: StaticCell<StackResources<SOCKETS>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        interfaces.station,
        embassy_net::Config::dhcpv4(DhcpConfig::default()),
        RESOURCES.init(StackResources::new()),
        u64::from(Rng::new().random()),
    );

    spawner.spawn(connection(controller).expect("WiFi task spawned once"));
    spawner.spawn(net(runner).expect("network task spawned once"));
    spawner.spawn(updates(stack, ota, boot).expect("update task spawned once"));
    stack
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    loop {
        if controller.connect_async().await.is_ok() {
            while controller.is_connected() {
                Timer::after(Duration::from_secs(2)).await;
            }
        }
        Timer::after(Duration::from_secs(2)).await;
    }
}

#[embassy_executor::task]
async fn net(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
async fn updates(stack: Stack<'static>, ota: &'static mut Ota, boot: Boot) {
    if boot == Boot::Trial {
        // An address is what proves the image can take the next update.
        let joined = select(stack.wait_config_up(), Timer::after(TRIAL)).await;
        let lease = brief_lease().await;
        if let Either::Second(()) = joined {
            let _ = ota.reject(&lease);
            software_reset();
        }
        let _ = ota.confirm(&lease);
    }

    stack.wait_config_up().await;
    let mut rx = [0; 4_096];
    let mut tx = [0; 512];
    loop {
        let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
        socket.set_timeout(Some(Duration::from_secs(30)));
        if socket.accept(OTA_PORT).await.is_err() {
            continue;
        }
        let installed = serve(&mut socket, ota).await;
        socket.close();
        let _ = socket.flush().await;
        if installed {
            Timer::after_millis(500).await;
            software_reset();
        }
    }
}

/// True once an image is installed and the board should restart so it is read.
async fn serve(socket: &mut TcpSocket<'_>, ota: &mut Ota) -> bool {
    let mut buffer = [0; HEAD_LIMIT];
    let outcome = match http::read_head(socket, &mut buffer).await {
        Ok((end, filled)) => match parse_upload(&buffer[..end], TOKEN) {
            Ok(upload) => match receive(socket, ota, upload, &buffer[end..filled]).await {
                Ok(()) => Ok(match upload {
                    Upload::App(_) => "installed, rebooting",
                    Upload::Web(_) => "web partition updated, rebooting",
                }),
                Err(error) => Err(error.response()),
            },
            Err(error) => Err((error.status(), error.message())),
        },
        Err(error) => Err((error.status(), error.message())),
    };
    let (status, message) = match outcome {
        Ok(message) => (200, message),
        Err(failure) => failure,
    };
    let mut body = String::<128>::new();
    let _ = writeln!(body, "{message}");
    let _ = http::respond(socket, status, "text/plain", body.as_bytes()).await;
    status == 200
}

async fn receive(
    socket: &mut TcpSocket<'_>,
    ota: &mut Ota,
    upload: Upload,
    early: &[u8],
) -> Result<(), OtaError> {
    let mut update = ota.begin(upload)?;
    http::read_body(
        socket,
        early,
        upload.length(),
        OtaError::Truncated,
        |bytes| update.write(bytes),
    )
    .await?;
    update.finish()
}

async fn brief_lease() -> Lease {
    loop {
        if let Some(lease) = Lease::try_brief() {
            return lease;
        }
        Timer::after(RETRY).await;
    }
}
