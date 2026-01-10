use core::future;

use embassy_time::{Duration, Instant, Timer};

pub struct Debouncer<T> {
    value: T,
    pending_value: Option<(T, Instant)>,
    debounce_time: Duration,
}

impl<T: PartialEq + Copy> Debouncer<T> {
    pub fn new(initial_value: T, debounce_time: Duration) -> Self {
        Self {
            value: initial_value,
            pending_value: None,
            debounce_time,
        }
    }

    /// Returns if the debounced value changed.
    pub fn process_data(&mut self, latest_data: T, now: Instant) -> bool {
        if latest_data == self.value {
            self.pending_value = None;
            false
        } else if let Some((pending_value, instant)) = self.pending_value
            && pending_value == latest_data
        {
            if (now - instant) > self.debounce_time {
                self.value = pending_value;
                self.pending_value = None;
                true
            } else {
                false
            }
        } else {
            self.pending_value = Some((latest_data, now));
            false
        }
    }

    /// Call this function along with the function you use to detect changes in the source that you're getting the value from.
    pub async fn wait(&mut self) {
        if let Some((_pending_value, instant)) = &self.pending_value {
            Timer::at(*instant + self.debounce_time).await;
        } else {
            future::pending::<()>().await;
        }
    }

    pub fn value(&self) -> T {
        self.value
    }
}
