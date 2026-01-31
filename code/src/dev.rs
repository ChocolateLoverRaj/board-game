#![no_std]
#![no_main]

use core::iter::{once, repeat};

use defmt::info;
use embassy_embedded_hal::{adapter::BlockingAsync, shared_bus::asynch::i2c::I2cDeviceWithConfig};
use embassy_executor::Spawner;
use embassy_futures::join::{join3, join4, join5};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Delay, Timer};
use esp_backtrace as _;
use esp_hal::{
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::{self, master::I2c},
    interrupt::software::SoftwareInterruptControl,
    peripherals::{GPIO7, RMT},
    rmt::Rmt,
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async, smart_led_buffer};
use lib::{RotaryButton, RotaryInput2};
use mcp23017_controller::Mcp23017;
use smart_leds::{RGB, SmartLedsWriteAsync, brightness};
use ssd1306::{I2CDisplayInterface, Ssd1306Async, prelude::*, size::DisplaySize128x64};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let _ = spawner;

    let p = esp_hal::init(Default::default());
    esp_alloc::heap_allocator!(size: 72 * 1024);
    // Needed for esp_rtos
    let timg0 = TimerGroup::new(p.TIMG0);
    let software_interrupt = SoftwareInterruptControl::new(p.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, software_interrupt.software_interrupt0);

    info!(
        "Hello from the secret hitler dev program. This will use all the peripherals to make sure they are working all at the same time."
    );

    spawner.spawn(leds(p.RMT, p.GPIO7)).unwrap();

    let i2c = Mutex::<CriticalSectionRawMutex, _>::new(
        I2c::new(p.I2C0, i2c::master::Config::default())
            .unwrap()
            .with_scl(p.GPIO5)
            .with_sda(p.GPIO6)
            .into_async(),
    );

    let mut mcp23017 = Mcp23017::new(
        I2cDeviceWithConfig::new(
            &i2c,
            i2c::master::Config::default().with_frequency(Rate::from_khz(400)),
        ),
        [false, false, false],
        BlockingAsync::new(Output::new(p.GPIO8, Level::High, OutputConfig::default())),
        Input::new(p.GPIO1, InputConfig::default().with_pull(Pull::Up)),
        Delay,
    );
    let (mcp23017_runner, ep) = mcp23017.run();
    let rotary_input = RotaryInput2::new();
    let (rotary_input_runner, mut rotary_input) = rotary_input.run(ep.B2, ep.B3);
    join5(
        mcp23017_runner,
        rotary_input_runner,
        async {
            loop {
                info!("rotary position: {}", rotary_input.value());
                rotary_input.watch().await;
            }
        },
        async {
            let i2c = I2cDeviceWithConfig::new(
                &i2c,
                i2c::master::Config::default().with_frequency(Rate::from_khz(400)),
            );
            let mut display = Ssd1306Async::new(
                I2CDisplayInterface::new(i2c),
                DisplaySize128x64,
                DisplayRotation::Rotate0,
            )
            .into_buffered_graphics_mode();
            display.init().await.unwrap();
            display.clear_buffer();
            display.flush().await.unwrap();
            let mut invert = false;
            loop {
                display.set_invert(invert).await.unwrap();
                Timer::after_secs(1).await;
                invert = !invert;
            }
        },
        async {
            let mut rotary_button = RotaryButton::new(ep.B1).await;
            loop {
                rotary_button.wait_until_press().await;
                info!("rotary button pressed");
            }
        },
    )
    .await;
}

#[embassy_executor::task]
async fn leds(rmt: RMT<'static>, pin: GPIO7<'static>) {
    const TOTAL_LEDS: usize = 64;
    let mut buffer = smart_led_buffer!(buffer_size_async(TOTAL_LEDS));
    let mut leds_adapter = SmartLedsAdapterAsync::new(
        Rmt::new(rmt, Rate::from_mhz(80))
            .unwrap()
            .into_async()
            .channel0,
        pin,
        &mut buffer,
    );

    let mut n = 0;
    loop {
        n += 1;
        if n == TOTAL_LEDS {
            n = 0;
        }
        leds_adapter
            .write(brightness(
                repeat(RGB::default())
                    .take(n)
                    .chain(once(RGB::new(255, 0, 0)))
                    .chain(repeat(RGB::default()).take(TOTAL_LEDS - n - 1)),
                5,
            ))
            .await
            .unwrap();
        Timer::after_millis(100).await;
    }
}
