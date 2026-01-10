#![no_std]
#![no_main]

use core::fmt::Write;

use defmt::info;
use embassy_executor::Spawner;
use embassy_futures::{join::*, select::*};
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    mono_font::{MonoTextStyleBuilder, iso_8859_16::FONT_7X14},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text, renderer::TextRenderer},
};
use esp_backtrace as _;
use esp_bootloader_esp_idf::partitions::{
    DataPartitionSubType, PARTITION_TABLE_MAX_LEN, PartitionType, read_partition_table,
};
use esp_hal::{
    efuse::Efuse,
    i2c::{self, master::I2c},
    interrupt::software::SoftwareInterruptControl,
    rmt::Rmt,
    rng::{Trng, TrngSource},
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async, smart_led_buffer};
use esp_println as _;
use esp_radio::ble::controller::BleConnector;
use esp_storage::FlashStorage;
use lib::{
    CONNECTIONS_MAX, DATA_BUFFER_LEN, EmbeddedStorageAsyncWrapper, L2CAP_CHANNELS_MAX,
    LED_BRIGHTNESS, MapStorageKey, MapStorageKeyValue, PSM_L2CAP_EXAMPLES, SERVICE_UUID,
};
use sequential_storage::{
    cache::NoCache,
    map::{MapConfig, MapStorage},
};
use smart_leds::{RGB8, SmartLedsWriteAsync};
use ssd1306::{
    I2CDisplayInterface, Ssd1306Async, prelude::DisplayRotation, prelude::*,
    size::DisplaySize128x64,
};
use trouble_host::prelude::*;

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

    let address: Address = Address::random(Efuse::mac_address());

    join(
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
                .font(&FONT_7X14)
                .text_color(BinaryColor::On)
                .build();
            struct DrawWriter<'a, D, S> {
                display: &'a mut D,
                position: Point,
                character_style: S,
            }
            impl<'a, D, S> DrawWriter<'a, D, S> {
                pub fn new(display: &'a mut D, position: Point, character_style: S) -> Self {
                    Self {
                        display,
                        position,
                        character_style,
                    }
                }
            }
            impl<D, S: TextRenderer + Clone> Write for DrawWriter<'_, D, S>
            where
                D: DrawTarget<Color = S::Color>,
            {
                fn write_str(&mut self, s: &str) -> core::fmt::Result {
                    self.position = Text::with_baseline(
                        s,
                        self.position,
                        self.character_style.clone(),
                        Baseline::Top,
                    )
                    .draw(self.display)
                    .map_err(|_| core::fmt::Error)?;
                    Ok(())
                }
            }
            let mut writer = DrawWriter::new(&mut display, Point::zero(), text_style);
            write!(writer, "{address}").unwrap();
            display.flush().await.unwrap();
            // Invert the display ocassionally to not cause burn-in
            let mut invert = false;
            loop {
                Timer::after(Duration::from_secs(60)).await;
                invert = !invert;
                display.set_invert(invert).await.unwrap();
            }
        },
        async {
            let mut flash = FlashStorage::new(p.FLASH);
            let mut pt_mem = [0; PARTITION_TABLE_MAX_LEN];
            let pt = read_partition_table(&mut flash, &mut pt_mem).unwrap();
            let nvs = pt
                .find_partition(PartitionType::Data(DataPartitionSubType::Nvs))
                .unwrap()
                .unwrap();
            let nvs_partition = nvs.as_embedded_storage(&mut flash);
            let map_config = MapConfig::new(0..nvs_partition.partition_size() as u32);
            let mut map_storage = MapStorage::<MapStorageKey, _, _>::new(
                EmbeddedStorageAsyncWrapper(nvs_partition),
                map_config,
                NoCache::new(),
            );

            let _trng_source = TrngSource::new(p.RNG, p.ADC1);
            let mut trng = Trng::try_new().unwrap();
            let radio = esp_radio::init().unwrap();
            let connector = BleConnector::new(&radio, p.BT, Default::default()).unwrap();
            let controller = ExternalController::<_, 20>::new(connector);

            // Hardcoded peripheral address
            info!("Our address = {:?}", address);

            let mut resources: HostResources<
                DefaultPacketPool,
                CONNECTIONS_MAX,
                L2CAP_CHANNELS_MAX,
            > = HostResources::new();
            // Because we have no buttons for yes/no on the fascist board (to confirm the pairing  pin),
            // The JustWorks pairing method will be used.
            // This method is susceptible to man in the middle attacks.
            // This risk is okay because this is just a game.
            let stack = trouble_host::new(controller, &mut resources)
                .set_random_address(address)
                .set_random_generator_seed(&mut trng)
                .set_io_capabilities(IoCapabilities::DisplayOnly);

            let mut data_buffer = [Default::default(); DATA_BUFFER_LEN];
            let mut iter = map_storage.fetch_all_items(&mut data_buffer).await.unwrap();
            while let Some((key, &value)) = iter.next(&mut data_buffer).await.unwrap() {
                let bond = MapStorageKeyValue { key, value }.into();
                info!("found existing bond: {:#?}", bond);
                stack.add_bond_information(bond).unwrap();
            }

            let Host {
                mut peripheral,
                mut runner,
                ..
            } = stack.build();

            let mut adv_data = [0; 31];
            let adv_data_len = AdStructure::encode_slice(
                &[AdStructure::Flags(
                    LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED,
                )],
                &mut adv_data[..],
            )
            .unwrap();

            let mut scan_data = [0; 31];
            let scan_data_len = AdStructure::encode_slice(
                &[
                    AdStructure::ShortenedLocalName(b"SH Game F"),
                    AdStructure::ServiceUuids128(&[SERVICE_UUID.as_raw().try_into().unwrap()]),
                ],
                &mut scan_data[..],
            )
            .unwrap();

            join(runner.run(), async {
                loop {
                    info!("Advertising, waiting for connection...");
                    let advertiser = peripheral
                        .advertise(
                            &Default::default(),
                            Advertisement::ConnectableScannableUndirected {
                                adv_data: &adv_data[..adv_data_len],
                                scan_data: &scan_data[..scan_data_len],
                            },
                        )
                        .await
                        .unwrap();
                    let conn = advertiser.accept().await.unwrap();
                    // If we set this to false, we can require the previously created bond to be used
                    // However, if the central deleted its saved bond, we would need a way to prompt the user
                    // To ask them if we should delete our saved bond and make a new one
                    // We don't have a way of prompting the user so we're not going to do that
                    conn.set_bondable(true).unwrap();
                    let bond = match select(
                        async {
                            loop {
                            let event = conn.next().await;
                            info!("Connection event: {:#?}", event);
                            match event {
                                ConnectionEvent::Disconnected { reason } => {
                                    panic!("BLE connection disconnected. reason: {:?}", reason);
                                }
                                ConnectionEvent::PairingComplete {
                                    security_level: _,
                                    bond,
                                } => {
                                    break bond;
                                }
                                ConnectionEvent::PassKeyDisplay(_) => {
                                    panic!(
                                        "fascist board is DisplayOnly so unexpected PassKeyDisplay"
                                    );
                                }
                                ConnectionEvent::PassKeyConfirm(_) => {
                                    panic!(
                                        "fascist board is DisplayOnly so unexpected PassKeyConfirm"
                                    );
                                }
                                ConnectionEvent::PassKeyInput => {
                                    panic!("this board is DisplayYesNo so unexpected PassKeyInput");
                                }
                                ConnectionEvent::PairingFailed(e) => {
                                    panic!("pairing failed: {e:?}");
                                }
                                _ => {
                                    panic!("unexpected connection event");
                                }
                            }
                        }
                        },
                        async {
                            // See https://github.com/embassy-rs/trouble/issues/522
                            loop {
                                if matches!(
                                    conn.security_level().unwrap(),
                                    SecurityLevel::Encrypted
                                        | SecurityLevel::EncryptedAuthenticated
                                ) {
                                    break;
                                }
                                Timer::after(Duration::from_millis(100)).await;
                            }
                        },
                    )
                    .await {
                        Either::First(bond) => bond,
                        Either::Second(_) => None
                    };
                    info!("bonded: {}", bond);
                    if let Some(bond) = bond {
                        info!("storing bond");
                        let MapStorageKeyValue { key, value } = MapStorageKeyValue::from(bond);
                        map_storage
                            .store_item(&mut [Default::default(); DATA_BUFFER_LEN], &key, &&value)
                            .await
                            .unwrap();
                    }

                    info!("Connection established");

                    let config = L2capChannelConfig {
                        mtu: Some(PAYLOAD_LEN as u16),
                        ..Default::default()
                    };
                    let mut ch1 =
                        L2capChannel::accept(&stack, &conn, &[PSM_L2CAP_EXAMPLES], &config)
                            .await
                            .unwrap();

                    info!("L2CAP channel accepted");

                    // Size of payload we're expecting
                    const PAYLOAD_LEN: usize = 27;
                    let mut rx = [0; PAYLOAD_LEN];
                    for i in 0..10 {
                        let len = ch1.receive(&stack, &mut rx).await.unwrap();
                        assert_eq!(len, rx.len());
                        assert_eq!(rx, [i; PAYLOAD_LEN]);
                    }

                    info!("L2CAP data received, echoing");
                    Timer::after(Duration::from_secs(1)).await;
                    for i in 0..10 {
                        let tx = [i; PAYLOAD_LEN];
                        ch1.send(&stack, &tx).await.unwrap();
                    }
                    info!("L2CAP data echoed");

                    Timer::after(Duration::from_secs(60)).await;
                }
            })
            .await;
        },
    )
    .await;
}
