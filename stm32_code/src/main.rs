#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::{
    Config, bind_interrupts,
    gpio::{Level, Output, Speed},
    rcc::{self, APBPrescaler, Hse, HseMode, LsConfig, Pll, PllMul, PllPreDiv, PllSource, Sysclk},
    time::mhz,
    usart::Uart,
};
use embassy_time::Timer;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USART1 => embassy_stm32::usart::InterruptHandler<embassy_stm32::peripherals::USART1>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
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
    info!("waiting to receive bytes");
    loop {
        let bytes_read = uart_rx.read(&mut buffer).await.unwrap();
        let bytes = &buffer[..bytes_read];
        info!("received bytes: {}", bytes);
        uart_tx.write(&[6, 7]).await.unwrap();
    }

    // let mut led = Output::new(p.PC13, Level::Low, Speed::Low);
    // info!("Turned LED on");
    // Timer::after_secs(1).await;
    // led.set_high();
    // info!("Turned LED off");

    core::future::pending().await
}
