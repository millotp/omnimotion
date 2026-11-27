//! WiFi station and DHCP: in the simulator the station is always associated with an address, and the
//! application's sockets reach the host directly. There is no update server.

use embassy_executor::Spawner;
use embassy_net::Stack;
use esp_hal::peripherals::WIFI;

#[path = "../../../../firmware/crates/omnimotion-board/src/net/http.rs"]
pub mod http;

pub fn start(_spawner: Spawner, _wifi: WIFI<'static>) -> Stack<'static> {
    omnimotion_sim::hal::console("wifi: connected (simulated), DHCP lease 192.168.4.2");
    Stack::simulated()
}
