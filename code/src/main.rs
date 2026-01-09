#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::{
    interrupt::software::SoftwareInterruptControl, rmt::Rmt, time::Rate, timer::timg::TimerGroup,
};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async, smart_led_buffer};
use esp_println as _;
use smart_leds::{RGB8, SmartLedsWriteAsync};

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

    info!("Welcome to the electronic board game Secret Hitler");

    // Some LEDS may be connected but not used
    const TOTAL_LEDS: usize = 64;
    // Index on a 8x8 grid
    fn i(x: usize, y: usize) -> usize {
        y * 8 + x
    }
    // No particular order to this as of now
    let aura_leds = [i(0, 0), i(6, 0), i(0, 2), i(6, 2), i(0, 4), i(6, 4)];
    // Each group of leds represents the LEDs for that policy slot
    let policy_leds = [
        [i(1, 1), i(1, 3)],
        [i(2, 1), i(2, 3)],
        [i(3, 1), i(3, 3)],
        [i(4, 1), i(4, 3)],
        [i(5, 1), i(5, 3)],
    ];
    // Order matters here
    let election_tracker_leds = [i(1, 6), i(2, 6), i(3, 6)];

    let ws2812_gpio = p.GPIO2;

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
    let led_brightness = 0.05;
    let aura_color = RGB8::new(255, 0, 255);
    let liberal_color = RGB8::new(0, 127, 255);
    let election_tracker_color = RGB8::new(0, 255, 0);

    // Turn on Aura LEDs. Make them purple for now to differentiate the boards.
    for aura_led_index in aura_leds {
        led_colors[aura_led_index] = aura_color.scale(led_brightness);
    }

    // Turn on the policy LEDs.
    for policy in policy_leds {
        for led_index in policy {
            led_colors[led_index] = liberal_color.scale(led_brightness);
        }
    }

    // Turn on the election tracker LEDs
    for election_tracker_led_index in election_tracker_leds {
        led_colors[election_tracker_led_index] = election_tracker_color.scale(led_brightness);
    }

    leds_adapter.write(led_colors).await.unwrap();
}
