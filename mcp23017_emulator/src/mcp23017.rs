use core::{iter::zip, mem, ops::Range};

use defmt::{Format, info, warn};
use embassy_stm32::{
    Peri,
    exti::{Channel, ExtiInput, InterruptHandler},
    gpio::{ExtiPin, Flex, Pin, Pull, Speed},
    interrupt::typelevel::Binding,
};
use embassy_time::{Duration, Instant};
use strum::{AsRefStr, Display, EnumCount, FromRepr, VariantNames};

#[derive(Debug, Format, Display, VariantNames, AsRefStr)]
#[strum(serialize_all = "snake_case")]
enum PinProperty {
    IoDirection,
    PullUpEnabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Format)]
enum IoDirection {
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

struct Mcp23017Gpio<'a> {
    pin: Flex<'a>,
    // /// Corresponds to the `IODIR` bit
    // io_direction: IoDirection,
    // /// Corresponds to the `IPOL` bit
    // invert: bool,
    // /// Corresponds to the `GPINTEN` bit
    // interrupt_on_change: bool,
    // /// Corresponds to the `DEFVAL` bit
    // value_to_compare: bool,
    // /// Corresponds to the `INTCON` bit
    // interrupt_control: bool,
    // /// Corresponds to the `GPPU` bit
    // pull_up_enabled: bool,
}

impl<'a> Mcp23017Gpio<'a> {
    pub fn new(pin: Peri<'a, impl Pin>) -> Self {
        Self {
            pin: Flex::new(pin),
            // io_direction: IoDirection::Input,
            // invert: false,
            // interrupt_on_change: false,
            // value_to_compare: false,
            // interrupt_control: false,
            // pull_up_enabled: false,
        }
    }
}

impl Mcp23017Gpio<'_> {
    // fn pull(&self) -> Pull {
    //     if self.pull_up_enabled {
    //         Pull::Up
    //     } else {
    //         Pull::None
    //     }
    // }

    // fn update_pin(&mut self) {
    //     match self.io_direction {
    //         IoDirection::Input => self.pin.set_as_input(self.pull()),
    //         IoDirection::Output => {
    //             self.pin.set_as_output(Speed::Low);
    //         }
    //     }
    // }

    // pub fn set_io_direction(&mut self, io_direction: IoDirection) {
    //     self.io_direction = io_direction;
    //     self.update_pin();
    // }

    // pub fn set_pull_up_enabled(&mut self, pull_up_enabled: bool) {
    //     info!("pull up enabled set to: {}", pull_up_enabled);
    //     self.pull_up_enabled = pull_up_enabled;
    //     self.update_pin();
    // }

    pub fn update_pin(&mut self, io_direction: IoDirection, pull_up: bool) {
        match io_direction {
            IoDirection::Input => {
                self.pin
                    .set_as_input(if pull_up { Pull::Up } else { Pull::None })
            }
            IoDirection::Output => {
                self.pin.set_as_output(Speed::Low);
            }
        }
    }
}

struct ResetPin<'a> {
    pin: ExtiInput<'a>,
    low_since: Option<Instant>,
}

impl<'a> ResetPin<'a> {
    pub fn new<T: ExtiPin + Pin>(
        pin: Peri<'a, T>,
        ch: Peri<'a, T::ExtiChannel>,
        irq: impl Binding<
            <<T as ExtiPin>::ExtiChannel as Channel>::IRQ,
            InterruptHandler<<<T as ExtiPin>::ExtiChannel as Channel>::IRQ>,
        >,
    ) -> Self {
        Self {
            pin: ExtiInput::new(pin, ch, Pull::Up, irq),
            low_since: None,
        }
    }
}

impl ResetPin<'_> {
    pub async fn wait_until_reset(&mut self) {
        loop {
            if let Some(low_since) = self.low_since {
                // From the data sheet
                let minimum_duration = Duration::from_micros(1);
                self.pin.wait_for_high().await;
                self.low_since = None;
                let low_duration = low_since.elapsed();
                if low_duration >= minimum_duration {
                    break;
                } else {
                    warn!(
                        "reset pin went low for {} us, which is not long enough to trigger a reset ({} us)",
                        low_duration.as_micros(),
                        minimum_duration
                    );
                }
            } else {
                self.pin.wait_for_low().await;
                self.low_since = Some(Instant::now());
            }
        }
    }
}

/// There are 8 GPIO pins for set A and set B
const N_GPIO_PINS_PER_SET: usize = 8;
const N_TOTAL_GPIO_PINS: usize = N_GPIO_PINS_PER_SET * AB::COUNT;

pub struct Mcp23017<'a> {
    gpio_pins: [Mcp23017Gpio<'a>; N_TOTAL_GPIO_PINS],
    /// If you can, directly use your micro controller's RESET pin.
    /// We can also emulate a RESET pin.
    reset: ResetPin<'a>,
    bank_mode: bool,
    sequential_mode: bool,
    selected_address: u8,
    /// Corresponds to the `IODIR` bit
    io_directions: [IoDirection; N_TOTAL_GPIO_PINS],
    pull_up_enabled: [bool; N_TOTAL_GPIO_PINS],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumCount)]
enum AB {
    A,
    B,
}

impl AB {
    pub fn set_index(&self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index / N_GPIO_PINS_PER_SET {
            0 => Self::A,
            1 => Self::B,
            _ => unreachable!(),
        }
    }

    pub fn range(&self) -> Range<usize> {
        self.set_index() * N_GPIO_PINS_PER_SET..(self.set_index() + 1) * N_GPIO_PINS_PER_SET
    }
}

impl Format for AB {
    fn format(&self, fmt: defmt::Formatter) {
        let str = match self {
            Self::A => "A",
            Self::B => "B",
        };
        defmt::write!(fmt, "{}", str);
    }
}

pub struct FormatPinIndex(usize);

impl Format for FormatPinIndex {
    fn format(&self, fmt: defmt::Formatter) {
        let letter = AB::from_index(self.0);
        let index_within_letter = self.0 % N_GPIO_PINS_PER_SET;
        defmt::write!(fmt, "{}{}", letter, index_within_letter);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumCount, FromRepr, Format)]
#[repr(u8)]
enum RegisterType {
    IODIR,
    IPOL,
    GPINTEN,
    DEFVAL,
    INTCON,
    IOCON,
    GPPU,
    INTF,
    INTCAP,
    GPIO,
    OLAT,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Format)]
struct Register {
    _type: RegisterType,
    ab: AB,
}

impl<'a> Mcp23017<'a> {
    pub fn new<T: ExtiPin + Pin>(
        gpio_a_0: Peri<'a, impl Pin>,
        gpio_a_1: Peri<'a, impl Pin>,
        gpio_a_2: Peri<'a, impl Pin>,
        gpio_a_3: Peri<'a, impl Pin>,
        gpio_a_4: Peri<'a, impl Pin>,
        gpio_a_5: Peri<'a, impl Pin>,
        gpio_a_6: Peri<'a, impl Pin>,
        gpio_a_7: Peri<'a, impl Pin>,
        gpio_b_0: Peri<'a, impl Pin>,
        gpio_b_1: Peri<'a, impl Pin>,
        gpio_b_2: Peri<'a, impl Pin>,
        gpio_b_3: Peri<'a, impl Pin>,
        gpio_b_4: Peri<'a, impl Pin>,
        gpio_b_5: Peri<'a, impl Pin>,
        gpio_b_6: Peri<'a, impl Pin>,
        gpio_b_7: Peri<'a, impl Pin>,
        int_a: Peri<'a, impl Pin>,
        int_b: Peri<'a, impl Pin>,
        reset_pin: Peri<'a, T>,
        reset_ch: Peri<'a, T::ExtiChannel>,
        reset_irq: impl Binding<
            <<T as ExtiPin>::ExtiChannel as Channel>::IRQ,
            InterruptHandler<<<T as ExtiPin>::ExtiChannel as Channel>::IRQ>,
        >,
    ) -> Self {
        let mut s = Self {
            gpio_pins: [
                Mcp23017Gpio::new(gpio_a_0),
                Mcp23017Gpio::new(gpio_a_1),
                Mcp23017Gpio::new(gpio_a_2),
                Mcp23017Gpio::new(gpio_a_3),
                Mcp23017Gpio::new(gpio_a_4),
                Mcp23017Gpio::new(gpio_a_5),
                Mcp23017Gpio::new(gpio_a_6),
                Mcp23017Gpio::new(gpio_a_7),
                Mcp23017Gpio::new(gpio_b_0),
                Mcp23017Gpio::new(gpio_b_1),
                Mcp23017Gpio::new(gpio_b_2),
                Mcp23017Gpio::new(gpio_b_3),
                Mcp23017Gpio::new(gpio_b_4),
                Mcp23017Gpio::new(gpio_b_5),
                Mcp23017Gpio::new(gpio_b_6),
                Mcp23017Gpio::new(gpio_b_7),
            ],
            reset: ResetPin::new(reset_pin, reset_ch, reset_irq),
            bank_mode: false,
            sequential_mode: false,
            selected_address: 0,
            io_directions: [IoDirection::Input; _],
            pull_up_enabled: [false; _],
        };
        s.reset();
        s
    }
}

enum AdvanceAddressMode {
    /// `IOCON.SEQOP = 0`, `IOCON.BANK = 1`
    Fixed,
    /// `IOCON.SEQOP = 0`, `IOCON.BANK = 0`
    Toggle,
    /// `IOCON.SEQOP = 1`
    Cycle,
}

fn advance_address(current_address: u8, mode: AdvanceAddressMode) -> u8 {
    match mode {
        AdvanceAddressMode::Fixed => current_address,
        AdvanceAddressMode::Toggle => {
            if current_address.is_multiple_of(2) {
                current_address + 1
            } else {
                current_address - 1
            }
        }
        AdvanceAddressMode::Cycle => {
            if current_address == (RegisterType::COUNT * 2 - 1) as u8 {
                0
            } else {
                current_address + 1
            }
        }
    }
}

/// If the address is invalid, returns `None`
fn register_from_addr(address: u8, bank_mode: bool) -> Option<Register> {
    Some({
        if bank_mode {
            if address < RegisterType::COUNT as u8 {
                Register {
                    ab: AB::A,
                    _type: RegisterType::from_repr(address)?,
                }
            } else {
                Register {
                    ab: AB::B,
                    _type: RegisterType::from_repr(address - RegisterType::COUNT as u8)?,
                }
            }
        } else {
            Register {
                ab: if address.is_multiple_of(2) {
                    AB::A
                } else {
                    AB::B
                },
                _type: RegisterType::from_repr(address / 2)?,
            }
        }
    })
}

impl Mcp23017<'_> {
    /// Init / reset everything to initial values
    pub fn reset(&mut self) {
        self.bank_mode = false;
        self.selected_address = 0;
        self.io_directions = [IoDirection::Input; _];
        self.pull_up_enabled = [false; _];
        for i in 0..N_TOTAL_GPIO_PINS {
            self.update_pin(i);
        }
    }

    fn advance_address_mode(&self) -> AdvanceAddressMode {
        if self.sequential_mode {
            AdvanceAddressMode::Cycle
        } else if !self.bank_mode {
            AdvanceAddressMode::Toggle
        } else {
            AdvanceAddressMode::Fixed
        }
    }

    fn advance_address(&mut self) {
        self.selected_address = advance_address(self.selected_address, self.advance_address_mode());
    }

    pub fn process_write_transaction(&mut self, bytes: &[u8]) {
        if let Some(&address) = bytes.first() {
            self.selected_address = address;
            for &byte in &bytes[1..] {
                if let Some(register) = register_from_addr(self.selected_address, self.bank_mode) {
                    self.write_register(register, byte);
                } else {
                    warn!(
                        "Attempted to write to invalid register address: {}. Not doing anything.",
                        self.selected_address
                    );
                }
                self.advance_address();
            }
        }
    }

    pub fn prepare_read_buffer(&mut self, buffer: &mut [u8]) {
        let mut address = self.selected_address;
        for byte in buffer {
            if let Some(register) = register_from_addr(address, self.bank_mode) {
                *byte = self.read_register(register);
            } else {
                warn!(
                    "Attempted to read to invalid register address: {}. Not doing anything.",
                    address
                );
            }
            address = advance_address(address, self.advance_address_mode());
        }
    }

    /// After transmitting bytes to the controller, call this function with the actual number of
    /// bytes read by the controller.
    pub fn confirm_bytes_read(&mut self, bytes_read: usize) {
        for _ in 0..bytes_read {
            self.advance_address();
        }
    }

    fn update_pin(&mut self, pin_index: usize) {
        self.gpio_pins[pin_index].update_pin(self.io_directions[pin_index], false);
    }

    /// Writes the register based on the saved address
    /// and updates the address pointer
    fn write_register(&mut self, register: Register, value: u8) {
        // info!("write {} to register {}", value, register);
        match register._type {
            RegisterType::IODIR => {
                let new_io_directions = {
                    let mut new_io_directions = self.io_directions;
                    for (index, io_direction) in new_io_directions[register.ab.range()]
                        .iter_mut()
                        .enumerate()
                    {
                        *io_direction = ((value & (1 << index)) != 0).into();
                    }
                    new_io_directions
                };
                let previous_io_directions =
                    mem::replace(&mut self.io_directions, new_io_directions);
                zip(previous_io_directions, self.io_directions)
                    .enumerate()
                    .filter_map(|(index, (io_direction, new_io_direction))| {
                        if new_io_direction != io_direction {
                            Some((index, new_io_direction))
                        } else {
                            None
                        }
                    })
                    .for_each(|(index, new_io_direction)| {
                        let property = PinProperty::IoDirection.as_ref();
                        info!(
                            "{}.{:015} = {}",
                            FormatPinIndex(index),
                            property,
                            new_io_direction
                        );
                        self.update_pin(index);
                    });
            }
            RegisterType::GPPU => {
                let new_io_directions = {
                    let mut new_io_directions = self.pull_up_enabled;
                    for (index, io_direction) in new_io_directions[register.ab.range()]
                        .iter_mut()
                        .enumerate()
                    {
                        *io_direction = (value & (1 << index)) != 0;
                    }
                    new_io_directions
                };
                let previous_io_directions =
                    mem::replace(&mut self.pull_up_enabled, new_io_directions);
                zip(previous_io_directions, self.pull_up_enabled)
                    .enumerate()
                    .filter_map(|(index, (io_direction, new_io_direction))| {
                        if new_io_direction != io_direction {
                            Some((index, new_io_direction))
                        } else {
                            None
                        }
                    })
                    .for_each(|(index, new_io_direction)| {
                        let property = PinProperty::PullUpEnabled.as_ref();
                        info!(
                            "{}.{:015} = {}",
                            FormatPinIndex(index),
                            property,
                            new_io_direction
                        );
                        self.update_pin(index);
                    });
            }
            register_type => todo!("write {register_type:?}"),
        }
    }

    /// Reads the register based on the saved address.
    /// Does not update the address pointer
    fn read_register(&mut self, register: Register) -> u8 {
        match register._type {
            RegisterType::IODIR => {
                let mut value = Default::default();
                for (i, io_direction) in self.io_directions[register.ab.range()]
                    .into_iter()
                    .cloned()
                    .enumerate()
                {
                    value |= u8::from(bool::from(io_direction)) << i;
                }
                value
            }
            RegisterType::GPPU => {
                let mut value = Default::default();
                for (i, io_direction) in self.pull_up_enabled[register.ab.range()]
                    .into_iter()
                    .cloned()
                    .enumerate()
                {
                    value |= u8::from(io_direction) << i;
                }
                value
            }
            RegisterType::GPIO => {
                let mut value = Default::default();
                for (i, pin) in self.gpio_pins[register.ab.range()].into_iter().enumerate() {
                    value |= u8::from(pin.pin.is_high()) << i;
                }
                value
            }
            register_type => todo!("read {register_type:?}"),
        }
    }

    /// Process any interrupts (and raise an interrupt accordingly).
    /// This future will never complete.
    /// The future is safe to cancel.
    ///
    /// Also handles the reset pin
    pub async fn run(&mut self) {
        loop {
            self.reset.wait_until_reset().await;
            info!("Received reset input. Resetting emulated MCP23017.");
            self.reset();
        }
    }
}
