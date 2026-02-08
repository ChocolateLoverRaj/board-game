#![no_std]
#![no_main]

use core::iter::{once, repeat_n};

use common::Request;
use defmt::{Debug2Format, info, warn};
use embassy_executor::Spawner;
use embassy_stm32::{
    Config, Peri, bind_interrupts,
    gpio::{Level, Output, Speed},
    peripherals::{DMA1_CH3, PA7, SPI1},
    rcc::{self, APBPrescaler, Hse, HseMode, Pll, PllMul, PllPreDiv, PllSource, Sysclk},
    spi::{self, Spi},
    time::{khz, mhz},
    usart::Uart,
};
use embassy_time::Timer;
use smart_leds::{RGB, SmartLedsWriteAsync, brightness};
use ws2812_async::{Grb, Ws2812};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USART1 => embassy_stm32::usart::InterruptHandler<embassy_stm32::peripherals::USART1>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) -> ! {
    let p = embassy_stm32::init({
        let mut config = Config::default();
        config.rcc = {
            let mut rcc = rcc::Config::new();
            rcc.hse = Some(Hse {
                freq: mhz(8),
                mode: HseMode::Oscillator,
            });
            rcc.pll = Some(Pll {
                prediv: PllPreDiv::DIV1,
                mul: PllMul::MUL9,
                src: PllSource::HSE,
            });
            rcc.apb1_pre = APBPrescaler::DIV2;
            rcc.apb2_pre = APBPrescaler::DIV1;
            rcc.sys = Sysclk::PLL1_P;
            rcc
        };
        config
    });

    // spawner.spawn(leds_task(p.SPI1, p.PA7, p.DMA1_CH3)).unwrap();
    let spi = Spi::new_txonly_nosck(p.SPI1, p.PA7, p.DMA1_CH3, {
        let mut config = spi::Config::default();
        config.frequency = khz(3800);
        config
    });
    const TOTAL_LEDS: usize = 64;
    let mut leds = Ws2812::<_, Grb, TOTAL_LEDS>::new(spi);

    let mut led = Output::new(p.PC13, Level::High, Speed::Low);

    let uart = Uart::new(p.USART1, p.PA10, p.PA9, Irqs, p.DMA1_CH4, p.DMA1_CH5, {
        let mut config = embassy_stm32::usart::Config::default();
        config.baudrate = 4_500_000;
        config.eager_reads = Some(1);
        config
    })
    .unwrap();
    let (mut uart_tx, uart_rx) = uart.split();
    let mut dma_buf = [Default::default(); 1024];
    let mut uart_rx = uart_rx.into_ring_buffered(&mut dma_buf);
    let mut buffer = [Default::default(); 1024];
    let mut buffer_bytes = 0;
    info!("waiting to receive bytes");
    loop {
        let new_bytes_read = uart_rx.read(&mut buffer[buffer_bytes..]).await.unwrap();
        {
            let new_bytes = &buffer[buffer_bytes..buffer_bytes + new_bytes_read];
            info!("received bytes: {}", new_bytes);
            buffer_bytes += new_bytes_read;
        }
        loop {
            let bytes = &mut buffer[..buffer_bytes];
            let zero_index = match bytes.iter().copied().position(|byte| byte == 0) {
                Some(zero_index) => zero_index,
                None => break,
            };
            let packet_len = zero_index + 1;
            match postcard::take_from_bytes_cobs::<Request>(&mut buffer[..packet_len]) {
                Ok((request, remaining_bytes)) => {
                    info!("Received request: {}", Debug2Format(&request));
                    match request {
                        Request::SetLed(state) => {
                            led.set_level(state.into());
                        }
                        Request::SetLeds(colors) => {
                            leds.write(colors).await.unwrap();
                        }
                    }
                }
                Err(e) => {
                    warn!("Error: {}", e);
                }
            }
            buffer.copy_within(packet_len..buffer_bytes, 0);
            buffer_bytes -= packet_len;
        }
        uart_tx.write(&[6, 7]).await.unwrap();
    }
}

#[embassy_executor::task]
async fn leds_task(
    spi: Peri<'static, SPI1>,
    pin: Peri<'static, PA7>,
    dma: Peri<'static, DMA1_CH3>,
) {
    let spi = Spi::new_txonly_nosck(spi, pin, dma, {
        let mut config = spi::Config::default();
        config.frequency = khz(3800);
        config
    });
    const TOTAL_LEDS: usize = 64;
    let mut leds = Ws2812::<_, Grb, TOTAL_LEDS>::new(spi);

    let mut n = 0;
    loop {
        n += 1;
        if n == TOTAL_LEDS {
            n = 0;
        }
        let alternate_color = if n.is_multiple_of(2) {
            RGB::new(255, 165, 0)
        } else {
            RGB::new(255, 50, 0)
        };
        leds.write(brightness(
            repeat_n(alternate_color, n)
                .chain(once(RGB::new(255, 0, 0)))
                .chain(repeat_n(alternate_color, TOTAL_LEDS - n - 1)),
            5,
        ))
        .await
        .unwrap();
        Timer::after_millis(100).await;
    }
}
