use embassy_time::{Duration, Instant};
use embedded_hal_async::digital::Wait;

pub struct ResetPin<T> {
    pin: T,
    low_since: Option<Instant>,
}

impl<T> ResetPin<T> {
    pub fn into_pin(self) -> T {
        self.pin
    }
}

impl<T: Wait> ResetPin<T> {
    pub fn new(pin: T) -> Self {
        Self {
            pin,
            low_since: None,
        }
    }
}

impl<T: Wait> ResetPin<T> {
    pub async fn wait_until_reset(&mut self) {
        loop {
            if let Some(low_since) = self.low_since {
                // From the data sheet
                let minimum_duration = Duration::from_micros(1);
                self.pin.wait_for_high().await.unwrap();
                self.low_since = None;
                let low_duration = low_since.elapsed();
                if low_duration >= minimum_duration {
                    break;
                } else {
                    #[cfg(feature = "defmt")]
                    defmt::warn!(
                        "reset pin went low for {} us, which is not long enough to trigger a reset ({} us)",
                        low_duration.as_micros(),
                        minimum_duration
                    );
                }
            } else {
                self.pin.wait_for_low().await.unwrap();
                self.low_since = Some(Instant::now());
            }
        }
    }
}
