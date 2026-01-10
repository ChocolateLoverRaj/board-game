use core::{mem, ops::Not};

use defmt::*;
use embassy_time::Instant;

#[derive(Debug, Format, Clone, Copy, PartialEq, Eq)]
pub struct RotaryPinsState {
    pub clk: bool,
    pub dt: bool,
}

#[derive(Debug, Format, Clone, Copy, PartialEq, Eq)]
enum RotaryPin {
    Clock,
    Dt,
}

#[derive(Debug, Format, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Clockwise,
    CounterClockwise,
}

impl Not for Direction {
    type Output = Self;

    fn not(self) -> Self::Output {
        match self {
            Self::Clockwise => Self::CounterClockwise,
            Self::CounterClockwise => Self::Clockwise,
        }
    }
}

impl RotaryPin {
    /// Returns +1 if clockwise and -1 if counter-clockwise
    pub fn leading_direction(&self) -> Direction {
        match self {
            Self::Clock => Direction::Clockwise,
            Self::Dt => Direction::CounterClockwise,
        }
    }
}

pub struct RotaryEncoder {
    state: RotaryPinsState,
    leading_pin: Option<RotaryPin>,
    last_changed: Instant,
}

impl RotaryEncoder {
    pub fn new(state: RotaryPinsState) -> Self {
        Self {
            state,
            leading_pin: None,
            last_changed: Instant::now(),
        }
    }

    pub fn process_data(&mut self, new_state: RotaryPinsState) -> Option<Direction> {
        let direction = if new_state != self.state {
            let now = Instant::now();
            let last_changed = mem::replace(&mut self.last_changed, now);
            trace!(
                "time between change: {} us",
                (now - last_changed).as_micros()
            );
            let clk_changed = new_state.clk != self.state.clk;
            let dt_changed = new_state.dt != self.state.dt;
            let changed_pin = match (clk_changed, dt_changed) {
                (true, false) => Some(RotaryPin::Clock),
                (false, true) => Some(RotaryPin::Dt),
                _ => None,
            };
            if let Some(changed_pin) = changed_pin {
                Some(if let Some(leading_pin) = self.leading_pin {
                    let change = if changed_pin != leading_pin {
                        // non-leading pin caught up
                        leading_pin.leading_direction()
                    } else {
                        // leading pin moved back
                        trace!("leading pin moved back");
                        !leading_pin.leading_direction()
                    };
                    self.leading_pin = None;
                    change
                } else {
                    // pin moved and is not a leading pin
                    trace!("new leading pin");
                    self.leading_pin = Some(changed_pin);
                    changed_pin.leading_direction()
                })
            } else {
                // Since both pins changed, we know that it moved, but we don't know which direction
                trace!("both changed");
                None
            }
        } else {
            None
        };
        self.state = new_state;
        direction
    }
}
