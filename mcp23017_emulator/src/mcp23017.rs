use core::{future::pending, iter::zip, mem, ops::Range};

use collect_array_ext_trait::CollectArray;
use embassy_futures::{
    select::{select, select_array},
    yield_now,
};
use embedded_hal::digital::PinState;
use embedded_hal_async::digital::Wait;
use strum::{AsRefStr, Display, EnumCount, FromRepr, VariantArray, VariantNames};

use crate::{
    InterruptMode, InterruptPin,
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
    InterruptControl,
    InputInverted,
    InterruptEnabled,
    CompareValue,
}

/// There are 8 GPIO pins for set A and set B
const N_GPIO_PINS_PER_SET: usize = 8;
const N_TOTAL_GPIO_PINS: usize = N_GPIO_PINS_PER_SET * AB::COUNT;

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumCount, VariantArray)]
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

    pub fn starting_index(&self) -> usize {
        self.set_index() * N_GPIO_PINS_PER_SET
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

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InterruptControl {
    /// Compare with `DEFVAL` register
    CompareWithConfiguredValue,
    CompareWithPreviousValue,
}

impl From<bool> for InterruptControl {
    fn from(value: bool) -> Self {
        match value {
            true => Self::CompareWithConfiguredValue,
            false => Self::CompareWithPreviousValue,
        }
    }
}

impl From<InterruptControl> for bool {
    fn from(value: InterruptControl) -> Self {
        match value {
            InterruptControl::CompareWithConfiguredValue => true,
            InterruptControl::CompareWithPreviousValue => false,
        }
    }
}

pub struct Mcp23017<P, I, R> {
    gpio_pins: [P; N_TOTAL_GPIO_PINS],
    interrupt_pins: [I; AB::COUNT],
    /// If you can, directly use your micro controller's RESET pin.
    /// We can also emulate a RESET pin.
    reset: ResetPin<R>,
    // IOCON settings
    /// IOCON.BANK
    bank_mode: bool,
    /// IOCON.MIRROR
    /// If set, an interrupt from any GPIO will trigger both interrupt pins
    mirror_interrupts: bool,
    /// IOCON.SEQOP
    sequential_mode: bool,
    /// IOCON.ODR
    int_mode: InterruptMode,
    /// IOCON.INTPOL
    int_active_state: PinState,

    selected_address: u8,
    /// Corresponds to the `IODIR` bit
    io_directions: [IoDirection; N_TOTAL_GPIO_PINS],
    pull_up_enabled: [bool; N_TOTAL_GPIO_PINS],
    output_latches: [PinState; N_TOTAL_GPIO_PINS],
    gpio_inverted: [bool; N_TOTAL_GPIO_PINS],
    int_enabled: [bool; N_TOTAL_GPIO_PINS],
    int_compare: [PinState; N_TOTAL_GPIO_PINS],
    interrupt_control: [InterruptControl; N_TOTAL_GPIO_PINS],
    /// Can only be cleared by reading the GPIO or captured pin state
    int_flags: [bool; N_TOTAL_GPIO_PINS],
    int_captured_value: [PinState; N_TOTAL_GPIO_PINS],
}

impl<P: GpioPin, I: InterruptPin, R: Wait> Mcp23017<P, I, R> {
    pub fn new(
        gpio_pins: [P; N_TOTAL_GPIO_PINS],
        interrupt_pins: [I; AB::COUNT],
        reset_pin: R,
    ) -> Self {
        let mut s = Self {
            gpio_pins,
            interrupt_pins,
            reset: ResetPin::new(reset_pin),
            bank_mode: false,
            mirror_interrupts: false,
            sequential_mode: false,
            int_mode: InterruptMode::ActiveDriver,
            int_active_state: PinState::Low,
            selected_address: 0,
            io_directions: [IoDirection::Input; _],
            pull_up_enabled: [false; _],
            output_latches: [PinState::Low; _],
            gpio_inverted: [false; _],
            int_enabled: [false; _],
            int_compare: [PinState::Low; _],
            interrupt_control: [InterruptControl::CompareWithPreviousValue; _],
            int_flags: [false; _],
            int_captured_value: [PinState::Low; _],
        };
        s.update_all_pins();
        s.update_interrupts();
        s
    }

    /// Init / reset everything to initial values
    pub fn reset(&mut self) {
        self.bank_mode = false;
        self.mirror_interrupts = false;
        self.sequential_mode = false;
        self.int_mode = InterruptMode::ActiveDriver;
        self.int_active_state = PinState::Low;
        self.selected_address = 0;
        self.io_directions = [IoDirection::Input; _];
        self.pull_up_enabled = [false; _];
        self.output_latches = [PinState::Low; _];
        self.gpio_inverted = [false; _];
        self.int_enabled = [false; _];
        self.int_compare = [PinState::Low; _];
        self.interrupt_control = [InterruptControl::CompareWithPreviousValue; _];
        self.int_flags = [false; _];
        self.int_captured_value = [PinState::Low; _];
        self.update_all_pins();
        self.update_interrupts();
    }

    fn update_all_pins(&mut self) {
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
            self.pull_up_enabled[pin_index],
            self.output_latches[pin_index],
        );
    }

    fn update_interrupts(&mut self) {
        let mut enable_interrupts = [false; AB::COUNT];
        for (i, set) in AB::VARIANTS.iter().enumerate() {
            if self.int_flags[set.range()].contains(&true) {
                enable_interrupts[i] = true;
            }
        }
        if self.mirror_interrupts && enable_interrupts.contains(&true) {
            enable_interrupts.fill(true);
        }
        for (i, interrupt_pin) in self.interrupt_pins.iter_mut().enumerate() {
            if enable_interrupts[i] {
                #[cfg(feature = "defmt")]
                defmt::trace!("enabling interrupt pin {}", i);
            }
            interrupt_pin.configure(
                self.int_mode,
                if enable_interrupts[i] {
                    self.int_active_state
                } else {
                    !self.int_active_state
                },
            );
        }
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
                            "{}.{:017} = {}",
                            FormatPinIndex(index),
                            property,
                            new_io_direction
                        );
                        self.update_pin(index);
                    });
            }
            RegisterType::GPPU => {
                let new_pull_up_enabled = {
                    let mut new_pull_up_enabled = self.pull_up_enabled;
                    for (index, pull_up_enabled) in new_pull_up_enabled[register.ab.range()]
                        .iter_mut()
                        .enumerate()
                    {
                        *pull_up_enabled = (value & (1 << index)) != 0;
                    }
                    new_pull_up_enabled
                };
                let previous_pull_up_enabled =
                    mem::replace(&mut self.pull_up_enabled, new_pull_up_enabled);
                zip(previous_pull_up_enabled, self.pull_up_enabled)
                    .enumerate()
                    .filter_map(|(index, (previous_pull_up_enabled, pull_up_enabled))| {
                        if pull_up_enabled != previous_pull_up_enabled {
                            Some((index, pull_up_enabled))
                        } else {
                            None
                        }
                    })
                    .for_each(|(index, new_pull_up_enabled)| {
                        let property = PinProperty::PullUpEnabled.as_ref();
                        #[cfg(feature = "defmt")]
                        defmt::info!(
                            "{}.{:017} = {}",
                            FormatPinIndex(index),
                            property,
                            new_pull_up_enabled
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
                            "{}.{:017} = {}",
                            FormatPinIndex(index),
                            property,
                            defmt::Debug2Format(&pin_state)
                        );
                        self.update_pin(index);
                    });
            }
            RegisterType::IPOL => {
                let new_gpio_inverted = {
                    let mut new_gpio_inverted = self.gpio_inverted;
                    for (index, gpio_inverted) in new_gpio_inverted[register.ab.range()]
                        .iter_mut()
                        .enumerate()
                    {
                        *gpio_inverted = (value & (1 << index)) != 0;
                    }
                    new_gpio_inverted
                };
                let previous_gpio_inverted =
                    mem::replace(&mut self.gpio_inverted, new_gpio_inverted);
                zip(previous_gpio_inverted, self.gpio_inverted)
                    .enumerate()
                    .filter_map(|(index, (previous_value, current_value))| {
                        if current_value != previous_value {
                            Some((index, current_value))
                        } else {
                            None
                        }
                    })
                    .for_each(|(index, new_value)| {
                        let property = PinProperty::InputInverted.as_ref();
                        #[cfg(feature = "defmt")]
                        defmt::info!("{}.{:017} = {}", FormatPinIndex(index), property, new_value);
                    });
            }
            RegisterType::GPINTEN => {
                let new_int_enabled = {
                    let mut new_int_enabled = self.int_enabled;
                    for (index, int_enabled) in
                        new_int_enabled[register.ab.range()].iter_mut().enumerate()
                    {
                        *int_enabled = (value & (1 << index)) != 0;
                    }
                    new_int_enabled
                };
                let previous_int_enabled = mem::replace(&mut self.int_enabled, new_int_enabled);
                zip(previous_int_enabled, self.int_enabled)
                    .enumerate()
                    .filter_map(|(index, (previous_value, current_value))| {
                        if current_value != previous_value {
                            Some((index, current_value))
                        } else {
                            None
                        }
                    })
                    .for_each(|(index, new_value)| {
                        let property = PinProperty::InterruptEnabled.as_ref();
                        #[cfg(feature = "defmt")]
                        defmt::info!("{}.{:017} = {}", FormatPinIndex(index), property, new_value);
                        self.update_pin(index);
                    });
            }
            RegisterType::DEFVAL => {
                let new_compare_register = {
                    let mut new_compare_register = self.int_compare;
                    for (index, compare_value) in new_compare_register[register.ab.range()]
                        .iter_mut()
                        .enumerate()
                    {
                        *compare_value = ((value & (1 << index)) != 0).into();
                    }
                    new_compare_register
                };
                let previous_compare_register =
                    mem::replace(&mut self.int_compare, new_compare_register);
                zip(previous_compare_register, self.int_compare)
                    .enumerate()
                    .filter_map(|(index, (previous_value, current_value))| {
                        if current_value != previous_value {
                            Some((index, current_value))
                        } else {
                            None
                        }
                    })
                    .for_each(|(index, pin_state)| {
                        let property = PinProperty::CompareValue.as_ref();
                        #[cfg(feature = "defmt")]
                        defmt::info!(
                            "{}.{:017} = {}",
                            FormatPinIndex(index),
                            property,
                            defmt::Debug2Format(&pin_state)
                        );
                    });
            }
            RegisterType::INTCON => {
                let new_interrupt_control = {
                    let mut new_interrupt_control = self.interrupt_control;
                    for (index, interrupt_control) in new_interrupt_control[register.ab.range()]
                        .iter_mut()
                        .enumerate()
                    {
                        *interrupt_control = ((value & (1 << index)) != 0).into();
                    }
                    new_interrupt_control
                };
                let previous_interrupt_control =
                    mem::replace(&mut self.interrupt_control, new_interrupt_control);
                zip(previous_interrupt_control, self.interrupt_control)
                    .enumerate()
                    .filter_map(|(index, (previous_value, current_value))| {
                        if current_value != previous_value {
                            Some((index, current_value))
                        } else {
                            None
                        }
                    })
                    .for_each(|(index, new_value)| {
                        let property = PinProperty::InterruptControl.as_ref();
                        #[cfg(feature = "defmt")]
                        defmt::info!("{}.{:017} = {}", FormatPinIndex(index), property, new_value);
                    });
            }
            RegisterType::IOCON => {
                self.bank_mode = (value & 1 << 7) != 0;
                self.mirror_interrupts = (value & 1 << 6) != 0;
                self.sequential_mode = (value & 1 << 5) != 0;
                // what is slew rate idk
                self.int_mode = ((value & 1 << 2) != 0).into();
                self.int_active_state = ((value & 1 << 1) != 0).into();
                self.update_interrupts();
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
                    value |= u8::from(bool::from(
                        match self.io_directions[i + register.ab.starting_index()] {
                            IoDirection::Output => {
                                self.output_latches[i + register.ab.starting_index()].into()
                            }
                            IoDirection::Input => pin.level(),
                        },
                    )) << i;
                }
                // The interrupt is cleared
                self.int_flags[register.ab.range()].fill(false);
                #[cfg(feature = "defmt")]
                defmt::warn!("cleared interrupts for {}", register.ab);
                self.update_interrupts();
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
            use embassy_futures::select::Either::*;
            match select(
                self.reset.wait_until_reset(),
                select_array({
                    self.gpio_pins
                        .iter_mut()
                        .enumerate()
                        .map(async |(i, pin)| {
                            // Only send interrupts for pins that don't already have the interrupt flag on
                            if self.int_enabled[i] && !self.int_flags[i] {
                                let compare_value = match self.interrupt_control[i] {
                                    InterruptControl::CompareWithConfiguredValue => {
                                        self.int_compare[i]
                                    }
                                    InterruptControl::CompareWithPreviousValue => {
                                        // the docs are unclear about what the "previous value" is
                                        // i'm going to assume it's the previously captured value
                                        self.int_captured_value[i]
                                    }
                                };
                                if pin.can_wait() {
                                    let level = !compare_value;
                                    #[cfg(feature = "defmt")]
                                    defmt::warn!(
                                        "pin {} waiting for level to be {}",
                                        i,
                                        defmt::Debug2Format(&level)
                                    );
                                    pin.wait_for_level(level).await;
                                } else {
                                    #[cfg(feature = "defmt")]
                                    defmt::warn!("pin {} can't wait. falling back to polling", i);
                                    loop {
                                        if pin.level() != compare_value {
                                            break;
                                        }
                                        yield_now().await;
                                    }
                                }
                                !compare_value
                            } else {
                                pending().await
                            }
                        })
                        .collect_array::<N_TOTAL_GPIO_PINS>()
                        .unwrap()
                }),
            )
            .await
            {
                First(()) => {
                    #[cfg(feature = "defmt")]
                    defmt::info!("Received reset input. Resetting emulated MCP23017.");
                    self.reset();
                }
                Second((level, index)) => {
                    #[cfg(feature = "defmt")]
                    defmt::info!(
                        "interrupt cuz pin {} changed to {}",
                        index,
                        defmt::Debug2Format(&level)
                    );
                    self.int_flags[index] = true;
                    self.int_captured_value[index] = level;
                    self.update_interrupts();
                }
            };
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
