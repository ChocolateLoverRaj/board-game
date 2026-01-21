use core::ops::{Index, IndexMut};

use defmt::{Format, info, warn};
use embassy_stm32::{
    Peri,
    exti::{Channel, ExtiInput, InterruptHandler},
    gpio::{ExtiPin, Flex, Pin, Pull, Speed},
    interrupt::typelevel::Binding,
};
use embassy_time::{Duration, Instant};
use strum::{EnumCount, FromRepr};

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
enum IoDirection {
    Output = 0b0,
    Input = 0b1,
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
    /// Corresponds to the `IODIR` bit
    io_direction: IoDirection,
    /// Corresponds to the `IPOL` bit
    invert: bool,
    /// Corresponds to the `GPINTEN` bit
    interrupt_on_change: bool,
    /// Corresponds to the `DEFVAL` bit
    value_to_compare: bool,
    /// Corresponds to the `INTCON` bit
    interrupt_control: bool,
    /// Corresponds to the `GPPU` bit
    pull_up_enabled: bool,
}

impl<'a> Mcp23017Gpio<'a> {
    pub fn new(pin: Peri<'a, impl Pin>) -> Self {
        Self {
            pin: Flex::new(pin),
            io_direction: IoDirection::Input,
            invert: false,
            interrupt_on_change: false,
            value_to_compare: false,
            interrupt_control: false,
            pull_up_enabled: false,
        }
    }
}

impl Mcp23017Gpio<'_> {
    fn pull(&self) -> Pull {
        if self.pull_up_enabled {
            Pull::Up
        } else {
            Pull::None
        }
    }

    fn update_pin(&mut self) {
        match self.io_direction {
            IoDirection::Input => self.pin.set_as_input(self.pull()),
            IoDirection::Output => {
                self.pin.set_as_output(Speed::Low);
            }
        }
    }

    pub fn set_io_direction(&mut self, io_direction: IoDirection) {
        self.io_direction = io_direction;
        self.update_pin();
    }

    pub fn set_pull_up_enabled(&mut self, pull_up_enabled: bool) {
        info!("pull up enabled set to: {}", pull_up_enabled);
        self.pull_up_enabled = pull_up_enabled;
        self.update_pin();
    }
}

struct Mcp23017PinSet<'a> {
    gpio: [Mcp23017Gpio<'a>; 8],
    int_a: Flex<'a>,
}

impl Mcp23017PinSet<'_> {
    pub fn reset(&mut self) {
        self.set_io_direction(0b1111_1111);
    }

    pub fn set_io_direction(&mut self, io_direction: u8) {
        for (index, gpio) in self.gpio.iter_mut().enumerate() {
            gpio.set_io_direction(((io_direction & 1 << index) != 0).into());
        }
    }

    pub fn get_io_direction(&self) -> u8 {
        let mut io_direction = Default::default();
        for (index, gpio) in self.gpio.iter().enumerate() {
            io_direction |= (u8::from(bool::from(gpio.io_direction)) & 1) << index;
        }
        io_direction
    }

    pub fn set_pull_up_enabled(&mut self, pull_up_enabled: u8) {
        for (index, gpio) in self.gpio.iter_mut().enumerate() {
            gpio.set_pull_up_enabled(((pull_up_enabled & 1 << index) != 0).into());
        }
    }

    pub fn get_pull_up_enabled(&self) -> u8 {
        let mut pull_up_enabled = Default::default();
        for (index, gpio) in self.gpio.iter().enumerate() {
            pull_up_enabled |= (u8::from(bool::from(gpio.pull_up_enabled)) & 1) << index;
        }
        info!("pull up enabled: {:#010b}", pull_up_enabled);
        pull_up_enabled
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

pub struct Mcp23017<'a> {
    a: Mcp23017PinSet<'a>,
    b: Mcp23017PinSet<'a>,
    /// If you can, directly use your micro controller's RESET pin.
    /// We can also emulate a RESET pin.
    reset: ResetPin<'a>,
    bank_mode: bool,
    sequential_mode: bool,
    selected_address: u8,
}

impl<'a> Index<AB> for Mcp23017<'a> {
    type Output = Mcp23017PinSet<'a>;

    fn index(&self, index: AB) -> &Self::Output {
        match index {
            AB::A => &self.a,
            AB::B => &self.b,
        }
    }
}

impl<'a> IndexMut<AB> for Mcp23017<'a> {
    fn index_mut(&mut self, index: AB) -> &mut Self::Output {
        match index {
            AB::A => &mut self.a,
            AB::B => &mut self.b,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Format)]
enum AB {
    A,
    B,
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
            a: Mcp23017PinSet {
                gpio: [
                    Mcp23017Gpio::new(gpio_a_0),
                    Mcp23017Gpio::new(gpio_a_1),
                    Mcp23017Gpio::new(gpio_a_2),
                    Mcp23017Gpio::new(gpio_a_3),
                    Mcp23017Gpio::new(gpio_a_4),
                    Mcp23017Gpio::new(gpio_a_5),
                    Mcp23017Gpio::new(gpio_a_6),
                    Mcp23017Gpio::new(gpio_a_7),
                ],
                int_a: Flex::new(int_a),
            },
            b: Mcp23017PinSet {
                gpio: [
                    Mcp23017Gpio::new(gpio_b_0),
                    Mcp23017Gpio::new(gpio_b_1),
                    Mcp23017Gpio::new(gpio_b_2),
                    Mcp23017Gpio::new(gpio_b_3),
                    Mcp23017Gpio::new(gpio_b_4),
                    Mcp23017Gpio::new(gpio_b_5),
                    Mcp23017Gpio::new(gpio_b_6),
                    Mcp23017Gpio::new(gpio_b_7),
                ],
                int_a: Flex::new(int_b),
            },
            reset: ResetPin::new(reset_pin, reset_ch, reset_irq),
            bank_mode: false,
            sequential_mode: false,
            selected_address: 0,
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
        self.a.reset();
        self.b.reset();
        self.bank_mode = false;
        self.selected_address = 0;
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

    /// Writes the register based on the saved address
    /// and updates the address pointer
    fn write_register(&mut self, register: Register, value: u8) {
        info!("write {} to register {}", value, register);
        match register._type {
            RegisterType::IODIR => self[register.ab].set_io_direction(value),
            RegisterType::GPPU => self[register.ab].set_pull_up_enabled(value),
            register_type => todo!("write {register_type:?}"),
        }
    }

    /// Reads the register based on the saved address.
    /// Does not update the address pointer
    fn read_register(&mut self, register: Register) -> u8 {
        info!("read register {}", register);
        match register._type {
            RegisterType::IODIR => self[register.ab].get_io_direction(),
            RegisterType::GPPU => self[register.ab].get_pull_up_enabled(),
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
