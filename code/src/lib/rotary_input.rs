use embassy_futures::select::select4;
use embassy_time::{Duration, Instant};
use esp_hal::gpio::{Input, InputConfig, InputPin, Level, Pull};

use crate::{Debouncer, Direction, RotaryEncoder, RotaryPinsState};

pub struct RotaryInput<'a> {
    dt: Input<'a>,
    dt_debounce: Debouncer<Level>,
    clk: Input<'a>,
    clk_debounce: Debouncer<Level>,
    rotary_encoder: RotaryEncoder,
}

impl<'a> RotaryInput<'a> {
    pub fn new(rotary_dt_gpio: impl InputPin + 'a, rotary_clk_gpio: impl InputPin + 'a) -> Self {
        let dt = Input::new(rotary_dt_gpio, InputConfig::default().with_pull(Pull::Up));
        let debounce_time = Duration::from_millis(1);
        let dt_debounce = Debouncer::new(dt.level(), debounce_time);
        let clk = Input::new(rotary_clk_gpio, InputConfig::default().with_pull(Pull::Up));
        let clk_debounce = Debouncer::new(clk.level(), debounce_time);
        let rotary_encoder = RotaryEncoder::new(RotaryPinsState {
            dt: dt_debounce.value() == Level::Low,
            clk: clk_debounce.value() == Level::Low,
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
                self.dt.wait_for_any_edge(),
                self.dt_debounce.wait(),
                self.clk.wait_for_any_edge(),
                self.clk_debounce.wait(),
            )
            .await;
            self.dt_debounce
                .process_data(self.dt.level(), Instant::now());
            self.clk_debounce
                .process_data(self.clk.level(), Instant::now());
            if let Some(direction) = self.rotary_encoder.process_data(RotaryPinsState {
                dt: self.dt_debounce.value() == Level::Low,
                clk: self.clk_debounce.value() == Level::Low,
            }) {
                break direction;
            }
        }
    }
}
