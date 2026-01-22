#![no_std]
#![no_main]
mod stm32_gpio_pin;

use defmt::{error, info, warn};
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_stm32::{
    bind_interrupts,
    exti::{self, ExtiInput},
    gpio::{Flex, Pull, Speed},
    i2c::{self, I2c, SlaveAddrConfig, SlaveCommandKind},
    interrupt,
    peripherals::{self},
    time::khz,
};
use mcp23017_emulator::Mcp23017;

use crate::stm32_gpio_pin::Stm32GpioPin;

use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    I2C1_EV => i2c::EventInterruptHandler<peripherals::I2C1>;
    I2C1_ER => i2c::ErrorInterruptHandler<peripherals::I2C1>;
    EXTI15_10  => exti::InterruptHandler<interrupt::typelevel::EXTI15_10>;
    EXTI9_5  => exti::InterruptHandler<interrupt::typelevel::EXTI9_5>;
});

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

    // ExtiInput::new(p.PA6, p.EXTI6, Pull::Down, Irqs);
    // ExtiInput::new(p.PA7, p.EXTI7, Pull::Down, Irqs);

    let mut mcp23017 = Mcp23017::new(
        [
            Stm32GpioPin::new_flex(Flex::new(p.PB0), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PA1), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PA2), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PA3), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PA4), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PA5), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PA6), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PB1), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PA7), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PA8), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PA9), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PA10), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PB11), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PB12), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PB13), Speed::Low),
            Stm32GpioPin::new_flex(Flex::new(p.PB14), Speed::Low),
        ],
        ExtiInput::new(p.PB15, p.EXTI15, Pull::Up, Irqs),
    );
    loop {
        info!("Ready to receive I2C commands");
        use embassy_futures::select::Either::*;
        let command = match select(mcp23017.run(), i2c.listen()).await {
            First(_) => unreachable!(),
            Second(command) => command,
        };
        let command = match command {
            Ok(command) => command,
            Err(e) => {
                warn!("I2C error: {}", e);
                continue;
            }
        };
        match command.kind {
            SlaveCommandKind::Read => {
                let mut buffer = [Default::default(); 32];
                mcp23017.prepare_read_buffer(&mut buffer);
                let use_sync_respond_to_read = true;
                let result = if use_sync_respond_to_read {
                    i2c.blocking_respond_to_read(&buffer)
                } else {
                    i2c.respond_to_read(&buffer).await
                };
                let bytes_transmitted = match result {
                    Err(e) => {
                        warn!("I2C error: {}", e);
                        continue;
                    }
                    Ok(bytes_transmitted) => bytes_transmitted,
                };
                if bytes_transmitted > buffer.len() {
                    error!(
                        "i2c controller read more bytes than we had prepared, so the controller read invalid data (zeros)."
                    );
                    continue;
                }
                mcp23017.confirm_bytes_read(bytes_transmitted);
            }
            SlaveCommandKind::Write => {
                let mut buffer = [Default::default(); 32];
                let use_sync_respond_to_write = true;
                let result = if use_sync_respond_to_write {
                    i2c.blocking_respond_to_write(&mut buffer)
                } else {
                    i2c.respond_to_write(&mut buffer).await
                };
                let bytes_received = match result {
                    Err(e) => {
                        warn!("I2C error: {}", e);
                        continue;
                    }
                    Ok(bytes_received) => bytes_received,
                };
                mcp23017.process_write_transaction(&buffer[..bytes_received]);
            }
        }
    }
}
