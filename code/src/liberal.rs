#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_futures::{join::*, select::*};
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::{
    mono_font::{MonoTextStyleBuilder, iso_8859_16::FONT_10X20},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};
use esp_backtrace as _;
use esp_hal::{
    gpio::{Input, InputConfig, Level, Pull},
    i2c::{self, master::I2c},
    interrupt::software::SoftwareInterruptControl,
    rmt::Rmt,
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async, smart_led_buffer};
use esp_println as _;
use esp_radio::ble::controller::BleConnector;
use smart_leds::{RGB8, SmartLedsWriteAsync};
use trouble_host::prelude::*;

use lib::{
    CONNECTIONS_MAX, Debouncer, L2CAP_CHANNELS_MAX, LED_BRIGHTNESS, PSM_L2CAP_EXAMPLES,
    RotaryEncoder, RotaryPinsState,
};
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
    esp_alloc::heap_allocator!(size: 72 * 1024);
    // Needed for esp_rtos
    let timg0 = TimerGroup::new(p.TIMG0);
    let software_interrupt = SoftwareInterruptControl::new(p.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, software_interrupt.software_interrupt0);

    info!("Welcome to the electronic board game Secret Hitler. This is the liberal board.");

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
    let i2c_scl_gpio = p.GPIO0;
    let i2c_sda_gpio = p.GPIO1;
    let rotary_clk_gpio = p.GPIO7;
    let rotary_dt_gpio = p.GPIO6;
    let rotary_sw_gpio = p.GPIO5;

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
    let aura_color = RGB8::new(255, 0, 255);
    let liberal_color = RGB8::new(0, 127, 255);
    let election_tracker_color = RGB8::new(0, 255, 0);

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

    // Turn on the election tracker LEDs
    for election_tracker_led_index in election_tracker_leds {
        led_colors[election_tracker_led_index] = election_tracker_color.scale(LED_BRIGHTNESS);
    }

    leds_adapter.write(led_colors).await.unwrap();

    join4(
        async {
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
                "Secret Hitler\nLiberal Board",
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
        },
        async {
            let mut switch = Input::new(rotary_sw_gpio, InputConfig::default().with_pull(Pull::Up));
            let mut debouncer = Debouncer::new(switch.level(), Duration::from_millis(1));
            loop {
                select(switch.wait_for_any_edge(), debouncer.wait()).await;
                let level_changed = debouncer.process_data(switch.level(), Instant::now());
                if level_changed {
                    info!("rotary button level: {}", debouncer.value());
                }
            }
        },
        async {
            let mut dt = Input::new(rotary_dt_gpio, InputConfig::default().with_pull(Pull::Up));
            let debounce_time = Duration::from_millis(1);
            let mut dt_debounce = Debouncer::new(dt.level(), debounce_time);
            let mut clk = Input::new(rotary_clk_gpio, InputConfig::default().with_pull(Pull::Up));
            let mut clk_debounce = Debouncer::new(clk.level(), debounce_time);
            let mut rotary_encoder = RotaryEncoder::new(RotaryPinsState {
                dt: dt_debounce.value() == Level::Low,
                clk: clk_debounce.value() == Level::Low,
            });
            loop {
                select4(
                    dt.wait_for_any_edge(),
                    dt_debounce.wait(),
                    clk.wait_for_any_edge(),
                    clk_debounce.wait(),
                )
                .await;
                dt_debounce.process_data(dt.level(), Instant::now());
                clk_debounce.process_data(clk.level(), Instant::now());
                if let Some(direction) = rotary_encoder.process_data(RotaryPinsState {
                    dt: dt_debounce.value() == Level::Low,
                    clk: clk_debounce.value() == Level::Low,
                }) {
                    info!("rotary direction: {}", direction);
                }
            }
        },
        async {
            let radio = esp_radio::init().unwrap();
            let connector = BleConnector::new(&radio, p.BT, Default::default()).unwrap();
            let controller = ExternalController::<_, 20>::new(connector);

            // Using a fixed "random" address can be useful for testing. In real scenarios, one would
            // use e.g. the MAC 6 byte array as the address (how to get that varies by the platform).
            let address: Address = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
            info!("Our address = {:?}", address);

            let mut resources: HostResources<
                DefaultPacketPool,
                CONNECTIONS_MAX,
                L2CAP_CHANNELS_MAX,
            > = HostResources::new();
            let stack = trouble_host::new(controller, &mut resources).set_random_address(address);
            let Host {
                mut central,
                mut runner,
                ..
            } = stack.build();

            // NOTE: Modify this to match the address of the peripheral you want to connect to.
            // Currently, it matches the address used by the peripheral examples
            let target: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);

            let config = ConnectConfig {
                connect_params: Default::default(),
                scan_config: ScanConfig {
                    filter_accept_list: &[(target.kind, &target.addr)],
                    ..Default::default()
                },
            };

            info!("Scanning for peripheral...");
            let _ = join(runner.run(), async {
                loop {
                    let conn = central.connect(&config).await.unwrap();
                    info!("Connected, creating l2cap channel");
                    const PAYLOAD_LEN: usize = 27;
                    let config = L2capChannelConfig {
                        mtu: Some(PAYLOAD_LEN as u16),
                        ..Default::default()
                    };
                    let mut ch1 = L2capChannel::create(&stack, &conn, PSM_L2CAP_EXAMPLES, &config)
                        .await
                        .unwrap();
                    info!("New l2cap channel created, sending some data!");
                    for i in 0..10 {
                        let tx = [i; PAYLOAD_LEN];
                        ch1.send(&stack, &tx).await.unwrap();
                    }
                    info!("Sent data, waiting for them to be sent back");
                    let mut rx = [0; PAYLOAD_LEN];
                    for i in 0..10 {
                        let len = ch1.receive(&stack, &mut rx).await.unwrap();
                        assert_eq!(len, rx.len());
                        assert_eq!(rx, [i; PAYLOAD_LEN]);
                    }

                    info!("Received successfully!");

                    Timer::after(Duration::from_secs(60)).await;
                }
            })
            .await;
        },
    )
    .await;
}
