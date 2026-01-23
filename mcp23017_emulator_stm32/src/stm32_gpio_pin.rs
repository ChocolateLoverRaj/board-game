use defmt::{info, warn};
use embassy_stm32::{
    Peri,
    exti::{Channel, ExtiInput, InterruptHandler},
    gpio::{ExtiPin, Flex, Level, Pull, Speed},
    interrupt::typelevel::Binding,
};
use mcp23017_emulator::{GpioPin, InterruptMode, InterruptPin, IoDirection, PinState};

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
    fn configure(&mut self, io_direction: IoDirection, pull_up_enabled: bool, level: PinState) {
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
                    pin.set_level(Level::from(bool::from(level)));
                    pin.set_as_output(*speed);
                }
                IoDirection::Input => {
                    pin.set_as_input(get_pull(pull_up_enabled));
                }
            },
        }
    }

    fn level(&mut self) -> PinState {
        bool::from(match &mut self._type {
            Stm32GpioPinType::ExtiInput { pin, pull: _ } => pin.get_level(),
            Stm32GpioPinType::Flex { pin, speed: _ } => pin.get_level(),
        })
        .into()
    }

    fn can_wait(&mut self) -> bool {
        match &self._type {
            Stm32GpioPinType::ExtiInput { pin: _, pull: _ } => true,
            Stm32GpioPinType::Flex { pin: _, speed: _ } => false,
        }
    }

    async fn wait_for_level(&mut self, level: PinState) {
        match &mut self._type {
            Stm32GpioPinType::ExtiInput { pin, pull: _ } => match level {
                PinState::High => pin.wait_for_high().await,
                PinState::Low => pin.wait_for_low().await,
            },
            Stm32GpioPinType::Flex { pin: _, speed: _ } => unreachable!(),
        }
    }
}

pub struct Stm32InterruptPin<'a> {
    pin: Flex<'a>,
    speed: Speed,
}

impl<'a> Stm32InterruptPin<'a> {
    pub fn new(pin: Flex<'a>, speed: Speed) -> Self {
        Self { pin, speed }
    }
}

impl InterruptPin for Stm32InterruptPin<'_> {
    fn configure(&mut self, mode: InterruptMode, level: PinState) {
        self.pin.set_level(bool::from(level).into());
        match mode {
            InterruptMode::OpenDrain => {
                self.pin.set_as_input_output(self.speed);
            }
            InterruptMode::ActiveDriver => {
                self.pin.set_as_output(self.speed);
            }
        }
    }
}
