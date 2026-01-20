#![no_std]
#![no_main]
use core::{ptr, slice};

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    gpio::{Flex, Input, Output, Pull},
    i2c::{self, I2c, SlaveAddrConfig, SlaveCommandKind},
    peripherals,
    time::khz,
};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    I2C1_EV => i2c::EventInterruptHandler<peripherals::I2C1>;
    I2C1_ER => i2c::ErrorInterruptHandler<peripherals::I2C1>;
});

struct Mcp23017Pins<'a> {
    gpio_a: [Flex<'a>; 8],
    int_a: Output<'a>,
    gpio_b: [Flex<'a>; 8],
    int_b: Output<'a>,
    /// If you can, directly use your micro controller's RESET pin.
    /// We can also emulate a RESET pin.
    reset: Input<'a>,
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());

    let mut i2c = I2c::new(p.I2C1, p.PB6, p.PB7, Irqs, p.DMA1_CH6, p.DMA1_CH7, {
        let mut config = i2c::Config::default();
        config.frequency = khz(400);
        config
    })
    .into_slave_multimaster(SlaveAddrConfig::basic({
        let base_address = 0x20;
        let least_significant_bits = 0b000;
        base_address | least_significant_bits
    }));

    let mut iocon_bank = false;
    let mut selected_address = 0_u8;
    loop {
        info!("Ready to receive I2C commands");
        let command = match i2c.blocking_listen() {
            Ok(command) => command,
            Err(e) => {
                warn!("I2C error: {}", e);
                continue;
            }
        };
        match command.kind {
            SlaveCommandKind::Read => {}
            SlaveCommandKind::Write => {
                // info!("I2C write command started");
                selected_address = {
                    let mut new_selected_address = Default::default();
                    match i2c.blocking_respond_to_write(slice::from_mut(&mut new_selected_address))
                    {
                        Ok(bytes) => {
                            info!("received {} bytes", bytes);
                            new_selected_address
                        }
                        Err(e) => {
                            warn!("I2C error: {}", e);
                            continue;
                        }
                    }
                };
                info!("Selected address: {}", selected_address);
                let value = {
                    let mut value = Default::default();
                    match i2c.blocking_respond_to_write(slice::from_mut(&mut value)) {
                        Ok(bytes) => {
                            info!("received {} bytes", bytes);
                            value
                        }
                        Err(e) => {
                            warn!("I2C error: {}", e);
                            continue;
                        }
                    }
                };
                info!("Value: {}", value);
            }
        }
    }
}
