use core::{iter::zip, mem, ops::Range};

use embedded_hal::digital::PinState;
use embedded_hal_async::digital::Wait;
use strum::{AsRefStr, Display, EnumCount, FromRepr, VariantNames};

use crate::{
    gpio_pin::{GpioPin, IoDirection},
    reset_pin::ResetPin,
};

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Display, VariantNames, AsRefStr)]
#[strum(serialize_all = "snake_case")]
enum PinProperty {
    IoDirection,
    PullUpEnabled,
    IoLatch,
}

/// There are 8 GPIO pins for set A and set B
const N_GPIO_PINS_PER_SET: usize = 8;
const N_TOTAL_GPIO_PINS: usize = N_GPIO_PINS_PER_SET * AB::COUNT;

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

#[cfg(feature = "defmt")]
impl defmt::Format for AB {
    fn format(&self, fmt: defmt::Formatter) {
        let str = match self {
            Self::A => "A",
            Self::B => "B",
        };
        defmt::write!(fmt, "{}", str);
    }
}

pub struct FormatPinIndex(usize);

#[cfg(feature = "defmt")]
impl defmt::Format for FormatPinIndex {
    fn format(&self, fmt: defmt::Formatter) {
        let letter = AB::from_index(self.0);
        let index_within_letter = self.0 % N_GPIO_PINS_PER_SET;
        defmt::write!(fmt, "{}{}", letter, index_within_letter);
    }
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumCount, FromRepr)]
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

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Register {
    _type: RegisterType,
    ab: AB,
}
pub struct Mcp23017<P, R> {
    gpio_pins: [P; N_TOTAL_GPIO_PINS],
    /// If you can, directly use your micro controller's RESET pin.
    /// We can also emulate a RESET pin.
    reset: ResetPin<R>,
    bank_mode: bool,
    sequential_mode: bool,
    selected_address: u8,
    /// Corresponds to the `IODIR` bit
    io_directions: [IoDirection; N_TOTAL_GPIO_PINS],
    pull_up_enabled: [bool; N_TOTAL_GPIO_PINS],
    output_latches: [PinState; N_TOTAL_GPIO_PINS],
}

impl<P: GpioPin, R: Wait> Mcp23017<P, R> {
    pub fn new(
        gpio_pins: [P; N_TOTAL_GPIO_PINS],
        // int_a: Peri<'a, impl Pin>,
        // int_b: Peri<'a, impl Pin>,
        reset_pin: R,
    ) -> Self {
        let mut s = Self {
            gpio_pins,
            reset: ResetPin::new(reset_pin),
            bank_mode: false,
            sequential_mode: false,
            selected_address: 0,
            io_directions: [IoDirection::Input; _],
            pull_up_enabled: [false; _],
            output_latches: [PinState::Low; _],
        };
        s.reset();
        s
    }

    /// Init / reset everything to initial values
    pub fn reset(&mut self) {
        self.bank_mode = false;
        self.selected_address = 0;
        self.io_directions = [IoDirection::Input; _];
        self.pull_up_enabled = [false; _];
        self.output_latches = [PinState::Low; _];
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
                    #[cfg(feature = "defmt")]
                    defmt::warn!(
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
                #[cfg(feature = "defmt")]
                defmt::warn!(
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
        self.gpio_pins[pin_index].configure(
            self.io_directions[pin_index],
            false,
            self.output_latches[pin_index],
        );
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
                        #[cfg(feature = "defmt")]
                        defmt::info!(
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
                        #[cfg(feature = "defmt")]
                        defmt::info!(
                            "{}.{:015} = {}",
                            FormatPinIndex(index),
                            property,
                            new_io_direction
                        );
                        self.update_pin(index);
                    });
            }
            RegisterType::OLAT | RegisterType::GPIO => {
                let new_io_directions = {
                    let mut new_output_latches = self.output_latches;
                    for (index, output_latch) in new_output_latches[register.ab.range()]
                        .iter_mut()
                        .enumerate()
                    {
                        *output_latch = ((value & (1 << index)) != 0).into();
                    }
                    new_output_latches
                };
                let previous_output_latches =
                    mem::replace(&mut self.output_latches, new_io_directions);
                zip(previous_output_latches, self.output_latches)
                    .enumerate()
                    .filter_map(|(index, (pin_state, new_pin_state))| {
                        if new_pin_state != pin_state {
                            Some((index, new_pin_state))
                        } else {
                            None
                        }
                    })
                    .for_each(|(index, pin_state)| {
                        let property = PinProperty::IoLatch.as_ref();
                        #[cfg(feature = "defmt")]
                        defmt::info!(
                            "{}.{:015} = {}",
                            FormatPinIndex(index),
                            property,
                            defmt::Debug2Format(&pin_state)
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
                for (i, pin) in (&mut self.gpio_pins[register.ab.range()])
                    .into_iter()
                    .enumerate()
                {
                    value |= u8::from(match self.io_directions[i] {
                        IoDirection::Output => self.output_latches[i].into(),
                        IoDirection::Input => pin.is_high(),
                    }) << i;
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
            #[cfg(feature = "defmt")]
            defmt::info!("Received reset input. Resetting emulated MCP23017.");
            self.reset();
        }
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
