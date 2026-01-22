use defmt::warn;
use embassy_stm32::{
    Peri,
    exti::{Channel, ExtiInput, InterruptHandler},
    gpio::{ExtiPin, Flex, Pull, Speed},
    interrupt::typelevel::Binding,
};
use mcp23017_emulator::{GpioPin, IoDirection};

fn get_pull(pull_up_enabled: bool) -> Pull {
    if pull_up_enabled {
        Pull::Up
    } else {
        Pull::None
    }
}

enum Stm32GpioPinType<'a> {
    ExtiInput { pin: ExtiInput<'a>, pull: Pull },
    Flex { pin: Flex<'a>, speed: Speed },
}

pub struct Stm32GpioPin<'a> {
    _type: Stm32GpioPinType<'a>,
}

impl<'a> Stm32GpioPin<'a> {
    pub fn new_exti<T: ExtiPin + embassy_stm32::gpio::Pin>(
        pin: Peri<'a, T>,
        ch: Peri<'a, T::ExtiChannel>,
        pull: Pull,
        irq: impl Binding<
            <<T as ExtiPin>::ExtiChannel as Channel>::IRQ,
            InterruptHandler<<<T as ExtiPin>::ExtiChannel as Channel>::IRQ>,
        >,
    ) -> Self {
        Self {
            _type: Stm32GpioPinType::ExtiInput {
                pin: ExtiInput::new(pin, ch, pull, irq),
                pull,
            },
        }
    }

    pub fn new_flex(pin: Flex<'a>, speed: Speed) -> Self {
        Self {
            _type: Stm32GpioPinType::Flex { pin, speed },
        }
    }
}

impl GpioPin for Stm32GpioPin<'_> {
    fn configure(&mut self, io_direction: IoDirection, pull_up_enabled: bool) {
        match &mut self._type {
            Stm32GpioPinType::ExtiInput { pin: _, pull } => {
                if io_direction == IoDirection::Input {
                    if *pull != get_pull(pull_up_enabled) {
                        warn!(
                            "Cannot set pull because ExtiInput's pull cannot be dynamically changed."
                        );
                    }
                }
                warn!("Tried to use input-only pin as output")
            }
            Stm32GpioPinType::Flex { pin, speed } => match io_direction {
                IoDirection::Output => {
                    pin.set_as_output(*speed);
                }
                IoDirection::Input => {
                    pin.set_as_input(get_pull(pull_up_enabled));
                }
            },
        }
    }

    fn is_high(&mut self) -> bool {
        match &mut self._type {
            Stm32GpioPinType::ExtiInput { pin, pull: _ } => pin.is_high(),
            Stm32GpioPinType::Flex { pin, speed: _ } => pin.is_high(),
        }
    }
}
