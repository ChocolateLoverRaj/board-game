#![no_std]
mod debouncer;
mod embedded_storage_async_wrapper;
pub mod liberal_renderer;
mod map_storage;
mod rotary_encoder;
mod rotary_input;
mod scale_rgb;

pub use debouncer::*;
pub use embedded_storage_async_wrapper::*;
pub use map_storage::*;
pub use rotary_encoder::*;
pub use rotary_input::*;
pub use scale_rgb::*;
use trouble_host::prelude::{Uuid, uuid};

pub const LED_BRIGHTNESS: f64 = 0.05;
pub const SERVICE_UUID: Uuid = uuid!("85d47eca-91e5-4ddb-9c23-0579415f46af");

/// Max number of connections
pub const CONNECTIONS_MAX: usize = 1;

/// Max number of L2CAP channels.
pub const L2CAP_CHANNELS_MAX: usize = 3; // Signal + att + CoC

// PSM from the dynamic range (0x0080-0x00FF) according to the Bluetooth
// Specification for L2CAP channels using LE Credit Based Flow Control mode.
// used for the BLE L2CAP examples.
//
// https://www.bluetooth.com/wp-content/uploads/Files/Specification/HTML/Core-60/out/en/host/logical-link-control-and-adaptation-protocol-specification.html#UUID-1ffdf913-7b8a-c7ba-531e-2a9c6f6da8fb
//
pub const PSM_L2CAP_EXAMPLES: u16 = 0x0081;
