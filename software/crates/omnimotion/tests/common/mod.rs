use embedded_hal_async::delay::DelayNs;

/// A deadline that has always already passed: whatever is not on the wire already is not coming.
pub struct Impatient;

impl DelayNs for Impatient {
    async fn delay_ns(&mut self, _ns: u32) {}
}
