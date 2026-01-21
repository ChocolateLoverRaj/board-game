#![no_std]
#![no_main]
mod mcp23017;

use crate::mcp23017::Mcp23017;
use defmt::{error, info, warn};
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_stm32::{
    bind_interrupts,
    exti::{self, ExtiInput},
    gpio::{ExtiPin, Pull},
    i2c::{self, I2c, SlaveAddrConfig, SlaveCommandKind},
    interrupt,
    peripherals::{self},
    time::khz,
};

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
        p.PA0, p.PA1, p.PA2, p.PA3, p.PA4, p.PA5, p.PA6, p.PA7, p.PB0, p.PB1, p.PB3, p.PB4, p.PB5,
        p.PB13, p.PB14, p.PA15, p.PA8, p.PB12, p.PB15, p.EXTI15, Irqs,
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

                // match i2c.transmit_byte_async(mcp23017.read_register()).await {
                //     Err(e) => {
                //         warn!("I2C error: {}", e);
                //         break;
                //     }
                //     Ok(TransmitResult::Acknowledged) => {
                //         info!("Responded to byte read, read transaction in progress.");
                //         mcp23017.advance_address();
                //     }
                //     Ok(TransmitResult::NotAcknowledged) => {
                //         info!("Sent byte, read transaction complete.");
                //         mcp23017.advance_address();
                //         break;
                //     }
                //     Ok(TransmitResult::Stopped | TransmitResult::Restarted) => {
                //         info!(
                //             "Transaction stopped, did not send byte. Not advancing address pointer."
                //         );
                //         break;
                //     }
                // }
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
                // let selected_address = match i2c.receive_byte_sync() {
                //     Err(e) => {
                //         warn!("I2C error: {}", e);
                //         continue;
                //     }
                //     Ok(ReceiveResult::Stopped | ReceiveResult::Restarted) => {
                //         warn!("I2C transaction complete before receiving address");
                //         continue;
                //     }
                //     Ok(ReceiveResult::Data(data)) => data,
                // };
                // mcp23017.set_address(selected_address);
                // info!("selected address: {}", selected_address);
                // loop {
                //     let value = match i2c.receive_byte_sync() {
                //         Err(e) => {
                //             warn!("I2C error: {}", e);
                //             break;
                //         }
                //         Ok(ReceiveResult::Stopped | ReceiveResult::Restarted) => {
                //             break;
                //         }
                //         Ok(ReceiveResult::Data(data)) => data,
                //     };
                //     info!("value: {}", value);
                //     mcp23017.write_register(value);
                // }
            }
        }
    }
}
