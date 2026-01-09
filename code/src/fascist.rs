#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_time::Timer;
use embedded_graphics::{
    mono_font::{MonoTextStyleBuilder, iso_8859_16::FONT_10X20},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};
use esp_backtrace as _;
use esp_hal::{
    i2c::{self, master::I2c},
    interrupt::software::SoftwareInterruptControl,
    rmt::Rmt,
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async, smart_led_buffer};
use esp_println as _;
use lib::LED_BRIGHTNESS;
use smart_leds::{RGB8, SmartLedsWriteAsync};
use ssd1306::{
    I2CDisplayInterface, Ssd1306Async, prelude::DisplayRotation, prelude::*,
    size::DisplaySize128x64,
};

esp_bootloader_esp_idf::esp_app_desc!();

trait ScaleRgb {
    fn scale(self, factor: f64) -> Self;
}

impl ScaleRgb for RGB8 {
    fn scale(self, factor: f64) -> Self {
        let Self { r, g, b } = self;
        Self::new(
            (r as f64 * factor) as u8,
            (g as f64 * factor) as u8,
            (b as f64 * factor) as u8,
        )
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let _ = spawner;

    let p = esp_hal::init(Default::default());

    // Needed for esp_rtos
    let timg0 = TimerGroup::new(p.TIMG0);
    let software_interrupt = SoftwareInterruptControl::new(p.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, software_interrupt.software_interrupt0);

    info!("Welcome to the electronic board game Secret Hitler. This is the fascist board.");

    // Some LEDS may be connected but not used
    const TOTAL_LEDS: usize = 64;
    // Index on a 8x8 grid
    fn i(x: usize, y: usize) -> usize {
        y * 8 + x
    }
    // No particular order to this as of now
    let aura_leds = [i(0, 0), i(7, 0), i(0, 2), i(7, 2), i(0, 4), i(7, 4)];
    // Each group of leds represents the LEDs for that policy slot
    let policy_leds = [
        [i(1, 1), i(1, 3)],
        [i(2, 1), i(2, 3)],
        [i(3, 1), i(3, 3)],
        [i(4, 1), i(4, 3)],
        [i(5, 1), i(5, 3)],
        [i(6, 1), i(6, 3)],
    ];

    let ws2812_gpio = p.GPIO2;
    let i2c_scl_gpio = p.GPIO0;
    let i2c_sda_gpio = p.GPIO1;

    let mut buffer = smart_led_buffer!(buffer_size_async(TOTAL_LEDS));
    let mut leds_adapter = SmartLedsAdapterAsync::new(
        Rmt::new(p.RMT, Rate::from_mhz(80))
            .unwrap()
            .into_async()
            .channel0,
        ws2812_gpio,
        &mut buffer,
    );
    let mut led_colors = [Default::default(); TOTAL_LEDS];

    // Scaling factor
    let aura_color = RGB8::new(255, 50, 50);
    let liberal_color = RGB8::new(255, 0, 0);

    // Turn on Aura LEDs
    for aura_led_index in aura_leds {
        led_colors[aura_led_index] = aura_color.scale(LED_BRIGHTNESS);
    }

    // Turn on the policy LEDs
    for policy in policy_leds {
        for led_index in policy {
            led_colors[led_index] = liberal_color.scale(LED_BRIGHTNESS);
        }
    }

    leds_adapter.write(led_colors).await.unwrap();

    // Turn on the OLED display
    let i2c = I2c::new(
        p.I2C0,
        i2c::master::Config::default().with_frequency(Rate::from_khz(400)),
    )
    .unwrap()
    .with_scl(i2c_scl_gpio)
    .with_sda(i2c_sda_gpio)
    .into_async();
    let mut display = Ssd1306Async::new(
        I2CDisplayInterface::new(i2c),
        DisplaySize128x64,
        DisplayRotation::Rotate0,
    )
    .into_buffered_graphics_mode();
    display.init().await.unwrap();
    let text_style = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(BinaryColor::On)
        .build();
    Text::with_baseline(
        "Secret Hitler\nFascist Board",
        Point::zero(),
        text_style,
        Baseline::Top,
    )
    .draw(&mut display)
    .unwrap();
    display.flush().await.unwrap();
    // Turn off the display to not cause burn-in
    Timer::after_secs(5).await;
    display.set_display_on(false).await.unwrap();
}
