use defmt::info;
use embassy_futures::select::{select, select4};
use embassy_time::{Duration, Instant};
use embedded_hal::digital::PinState;
use embedded_hal_async::digital::Wait;
use mcp23017_controller::{Input, Mcp23017};

use crate::{Debouncer, Direction, RotaryEncoder, RotaryPinsState};

pub struct RotaryInput<'a, ResetPin, I2c, InterruptPin> {
    dt: Input<'a, ResetPin, I2c, InterruptPin>,
    dt_debounce: Debouncer<PinState>,
    clk: Input<'a, ResetPin, I2c, InterruptPin>,
    clk_debounce: Debouncer<PinState>,
    rotary_encoder: RotaryEncoder,
}

impl<'a, ResetPin, I2c: embedded_hal_async::i2c::I2c, InterruptPin: Wait>
    RotaryInput<'a, ResetPin, I2c, InterruptPin>
{
    pub async fn new(
        mcp23017: &'a Mcp23017<ResetPin, I2c, InterruptPin>,
        dt_gpio: usize,
        clk_gpio: usize,
    ) -> Self {
        let debounce_time = Duration::from_millis(1);
        let dt = mcp23017.input(dt_gpio, true).await.unwrap();
        let dt_debounce = Debouncer::new(dt.last_known_state(), debounce_time);
        let clk = mcp23017.input(clk_gpio, true).await.unwrap();
        let clk_debounce = Debouncer::new(clk.last_known_state(), debounce_time);
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
                self.dt.wait_for_change(),
                self.dt_debounce.wait(),
                self.clk.wait_for_change(),
                self.clk_debounce.wait(),
            )
            .await;
            self.dt_debounce
                .process_data(self.dt.last_known_state(), Instant::now());
            self.clk_debounce
                .process_data(self.clk.last_known_state(), Instant::now());
            if let Some(direction) = self.rotary_encoder.process_data(RotaryPinsState {
                dt: self.dt_debounce.value() == PinState::Low,
                clk: self.clk_debounce.value() == PinState::Low,
            }) {
                break direction;
            }
        }
    }
}

pub struct RotaryButton<'a, ResetPin, I2c, InterruptPin> {
    switch: Input<'a, ResetPin, I2c, InterruptPin>,
    debouncer: Debouncer<PinState>,
}

impl<'a, ResetPin, I2c: embedded_hal_async::i2c::I2c, InterruptPin: Wait>
    RotaryButton<'a, ResetPin, I2c, InterruptPin>
{
    pub async fn new(mcp23017: &'a Mcp23017<ResetPin, I2c, InterruptPin>, sw_gpio: usize) -> Self {
        let switch = mcp23017.input(sw_gpio, true).await.unwrap();
        let debouncer = Debouncer::new(switch.last_known_state(), Duration::from_millis(1));
        Self { switch, debouncer }
    }

    pub async fn wait_until_press(&mut self) {
        loop {
            select(self.switch.wait_for_change(), self.debouncer.wait()).await;
            let level_changed = self
                .debouncer
                .process_data(self.switch.last_known_state(), Instant::now());
            if level_changed && self.debouncer.value() == PinState::Low {
                break;
            }
        }
    }
}
