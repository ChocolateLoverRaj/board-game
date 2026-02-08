#![no_std]

use serde::{Deserialize, Serialize};
use smart_leds::RGB;
#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    SetLed(bool),
    #[serde(with = "serde_arrays")]
    SetLeds([RGB<u8>; 64]),
}
