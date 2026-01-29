use core::any::Any;

use embassy_futures::select::{select, select4};
use embassy_time::{Duration, Instant};
use embedded_hal::digital::PinState;
use mcp23017_controller::{Pin, mode::Watch};

use crate::{Debouncer, Direction, RotaryEncoder, RotaryPinsState};

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
