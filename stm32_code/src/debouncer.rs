use core::future;

use embassy_time::{Duration, Instant, Timer};

pub struct Debouncer<T> {
    value: Option<T>,
    pending_value: Option<(T, Instant)>,
    debounce_time: Duration,
}

impl<T: PartialEq> Debouncer<T> {
    pub fn new(debounce_time: Duration) -> Self {
        Self {
            value: None,
            pending_value: None,
            debounce_time,
        }
    }

    /// Returns if the debounced value changed.
    pub fn process_data(&mut self, latest_data: T, now: Instant) -> Option<&T> {
        if Some(&latest_data) == self.value.as_ref() {
            self.pending_value = None;
            None
        } else if let Some((pending_value, instant)) = &self.pending_value
            && pending_value == &latest_data
        {
            if (now - *instant) >= self.debounce_time {
                let new_value = self.value.insert(self.pending_value.take().unwrap().0);
                Some(new_value)
            } else {
                None
            }
        } else {
            self.pending_value = Some((latest_data, now));
            None
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

    /// Gets the last processed value.
    /// It may be the pending value and not yet considered "stable".
    pub fn maybe_stable_value(&self) -> Option<&T> {
        self.pending_value
            .as_ref()
            .map(|(value, _)| value)
            .or(self.value.as_ref())
    }

    pub fn stable_value(&self) -> Option<&T> {
        self.value.as_ref()
    }

    // pub fn value(&self) -> T {
    //     self.value
    // }
}
