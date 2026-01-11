use core::num::NonZero;

use embassy_time::Duration;

/// Auto-connect to the last paired peripheral
pub const AUTO_CONNECT: bool = true;
/// If set, store bond info and give a warning if we connect to a peripheral with previously stored bond info,
/// but the peripheral does not have the previously saved bond info
/// (which could indicate a man in the middle attack).
/// Also, the number of bonds to store.
pub const SAVE_BOND_INFO: Option<NonZero<usize>> = None;
/// Invert the display every once in a while to reduce burn in.
/// I'm not sure whether this actually reduces burn-in
/// or if it just makes all pixels burned in more evenly.
/// Either way it preserves the screen quality over time
pub const INVERT_SCREEN_INTERVAL: Duration = Duration::from_secs(2 * 60);
