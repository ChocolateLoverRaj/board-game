#![no_std]

use defmt::Format;
use heapless::Vec;
use mfrc522::Uid;
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
    WatchNfc(bool),
}

pub const MAX_NFC_READERS: usize = 6;

#[derive(Debug, Format, Serialize, Deserialize)]
pub enum Event {
    SoftResetComplete,
    RotarySwitch(bool),
    RotaryEncoder(i64),
    Nfc(Vec<Option<Uid>, MAX_NFC_READERS>),
}
