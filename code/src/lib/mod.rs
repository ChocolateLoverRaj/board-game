#![no_std]
mod debouncer;
mod rotary_encoder;

pub use debouncer::*;
pub use rotary_encoder::*;

pub const LED_BRIGHTNESS: f64 = 0.05;
