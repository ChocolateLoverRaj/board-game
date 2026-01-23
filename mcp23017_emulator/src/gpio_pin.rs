pub use embedded_hal::digital::PinState;

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoDirection {
    Output,
    Input,
}

impl From<bool> for IoDirection {
    fn from(value: bool) -> Self {
        if value {
            IoDirection::Input
        } else {
            IoDirection::Output
        }
    }
}

impl From<IoDirection> for bool {
    fn from(value: IoDirection) -> Self {
        match value {
            IoDirection::Output => false,
            IoDirection::Input => true,
        }
    }
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptMode {
    OpenDrain,
    ActiveDriver,
}

impl From<bool> for InterruptMode {
    fn from(value: bool) -> Self {
        match value {
            true => Self::OpenDrain,
            false => Self::ActiveDriver,
        }
    }
}

impl From<InterruptMode> for bool {
    fn from(value: InterruptMode) -> Self {
        match value {
            InterruptMode::OpenDrain => true,
            InterruptMode::ActiveDriver => false,
        }
    }
}

pub trait GpioPin {
    fn configure(&mut self, io_direction: IoDirection, pull_up_enabled: bool, level: PinState);
    /// This function will not be called if this pin is configured to be in output mode.
    fn level(&mut self) -> PinState;
    /// Returns if the pin is capable of receiving interrupts (in input mode).
    fn can_wait(&mut self) -> bool;
    /// Returns when the pin's level becomes the specified level.
    /// This function will never be called if `can_wait` returns `false`.
    fn wait_for_level(&mut self, level: PinState) -> impl Future<Output = ()>;
}

pub trait InterruptPin {
    fn configure(&mut self, mode: InterruptMode, level: PinState);
}
