use core::any::Any;

use embassy_futures::select::{select, select4};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant};
use embedded_hal::digital::PinState;
use mcp23017_controller::{Pin, mode::Watch};

use crate::{Debouncer, Direction, RotaryEncoder, RotaryPinsState, rotary_encoder};

pub struct RotaryInput<'a> {
    dt: Pin<'a, Watch>,
    dt_debounce: Debouncer<PinState>,
    clk: Pin<'a, Watch>,
    clk_debounce: Debouncer<PinState>,
    rotary_encoder: RotaryEncoder,
}

impl<'a> RotaryInput<'a> {
    pub async fn new(dt: Pin<'a, impl Any>, clk: Pin<'a, impl Any>) -> Self {
        let debounce_time = Duration::from_millis(1);
        let mut dt = dt.into_watch(true).await;
        let dt_debounce = Debouncer::new(dt.state().await, debounce_time);
        let mut clk = clk.into_watch(true).await;
        let clk_debounce = Debouncer::new(clk.state().await, debounce_time);
        let rotary_encoder = RotaryEncoder::new(RotaryPinsState {
            dt: dt_debounce.value() == PinState::Low,
            clk: clk_debounce.value() == PinState::Low,
        });
        Self {
            dt,
            dt_debounce,
            clk,
            clk_debounce,
            rotary_encoder,
        }
    }

    pub async fn next(&mut self) -> Direction {
        loop {
            select4(
                self.dt.watch(),
                self.dt_debounce.wait(),
                self.clk.watch(),
                self.clk_debounce.wait(),
            )
            .await;
            self.dt_debounce
                .process_data(self.dt.state().await, Instant::now());
            self.clk_debounce
                .process_data(self.clk.state().await, Instant::now());
            if let Some(direction) = self.rotary_encoder.process_data(RotaryPinsState {
                dt: self.dt_debounce.value() == PinState::Low,
                clk: self.clk_debounce.value() == PinState::Low,
            }) {
                break direction;
            }
        }
    }
}

pub struct RotaryInput2 {
    signal: Signal<CriticalSectionRawMutex, i64>,
}

impl RotaryInput2 {
    pub fn new() -> Self {
        Self {
            signal: Signal::new(),
        }
    }

    pub fn run(
        &self,
        dt: Pin<'_, impl Any>,
        clk: Pin<'_, impl Any>,
    ) -> (impl Future<Output = ()>, RotaryInput2Receiver<'_>) {
        (
            async {
                let debounce_time = Duration::from_millis(1);
                let mut dt = dt.into_watch(true).await;
                let mut dt_debounce = Debouncer::new(dt.state().await, debounce_time);
                let mut clk = clk.into_watch(true).await;
                let mut clk_debounce = Debouncer::new(clk.state().await, debounce_time);
                let mut rotary_encoder = RotaryEncoder::new(RotaryPinsState {
                    dt: dt_debounce.value() == PinState::Low,
                    clk: clk_debounce.value() == PinState::Low,
                });
                let mut value = Default::default();
                loop {
                    select4(
                        dt.watch(),
                        dt_debounce.wait(),
                        clk.watch(),
                        clk_debounce.wait(),
                    )
                    .await;
                    dt_debounce.process_data(dt.state().await, Instant::now());
                    clk_debounce.process_data(clk.state().await, Instant::now());
                    if let Some(direction) = rotary_encoder.process_data(RotaryPinsState {
                        dt: dt_debounce.value() == PinState::Low,
                        clk: clk_debounce.value() == PinState::Low,
                    }) {
                        value += match direction {
                            Direction::Clockwise => 1,
                            Direction::CounterClockwise => -1,
                        };
                        self.signal.signal(value);
                    }
                }
            },
            RotaryInput2Receiver {
                signal: &self.signal,
                value: Default::default(),
            },
        )
    }
}
pub struct RotaryInput2Receiver<'a> {
    signal: &'a Signal<CriticalSectionRawMutex, i64>,
    value: i64,
}
impl RotaryInput2Receiver<'_> {
    pub fn value(&self) -> i64 {
        self.value
    }
    pub async fn watch(&mut self) {
        loop {
            let new_value = self.signal.wait().await;
            if new_value != self.value {
                self.value = new_value;
                break;
            }
        }
    }
}
pub struct RotaryButton<'a> {
    switch: Pin<'a, Watch>,
    debouncer: Debouncer<PinState>,
}

impl<'a> RotaryButton<'a> {
    pub async fn new(switch: Pin<'a, impl Any>) -> Self {
        let mut switch = switch.into_watch(true).await;
        let debouncer = Debouncer::new(switch.state().await, Duration::from_millis(1));
        Self { switch, debouncer }
    }

    pub async fn wait_until_press(&mut self) {
        loop {
            select(self.switch.watch(), self.debouncer.wait()).await;
            let level_changed = self
                .debouncer
                .process_data(self.switch.state().await, Instant::now());
            if level_changed && self.debouncer.value() == PinState::Low {
                break;
            }
        }
    }
}
