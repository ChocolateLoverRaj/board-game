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

pub trait GpioPin {
    fn configure(&mut self, io_direction: IoDirection, pull_up_enabled: bool);
    fn is_high(&mut self) -> bool;
}
