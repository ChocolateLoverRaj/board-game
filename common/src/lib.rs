#![no_std]

use defmt::Format;
use serde::{Deserialize, Serialize};
use smart_leds::RGB;

#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    SoftReset,
    SetLed(bool),
    #[serde(with = "serde_arrays")]
    SetLeds([RGB<u8>; 64]),
    WatchRotarySwitch(bool),
    WatchRotaryEncoder(bool),
}

#[derive(Debug, Format, Serialize, Deserialize)]
pub enum Event {
    SoftResetComplete,
    RotarySwitch(bool),
    RotaryEncoder(i64),
}
