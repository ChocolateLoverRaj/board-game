#![no_std]
#![no_main]

use core::iter::repeat;

use defmt::info;
use embassy_embedded_hal::{adapter::BlockingAsync, shared_bus::asynch::i2c::I2cDeviceWithConfig};
use embassy_executor::Spawner;
use embassy_futures::join::*;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::Delay;
use esp_backtrace as _;
use esp_bootloader_esp_idf::partitions::{
    DataPartitionSubType, PARTITION_TABLE_MAX_LEN, PartitionType, read_partition_table,
};
use esp_hal::{
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::{self, master::I2c},
    interrupt::software::SoftwareInterruptControl,
    rmt::Rmt,
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async, smart_led_buffer};
use esp_println as _;
use esp_storage::FlashStorage;
use game_pure::{BleAction, ConnectState, GameState};
use mcp23017_controller::Mcp23017;
use sequential_storage::{
    cache::NoCache,
    map::{MapConfig, MapStorage},
};
use smart_leds::{RGB8, SmartLedsWriteAsync};
use trouble_host::prelude::*;

use lib::{
    Direction, LED_BRIGHTNESS, LIBERAL_DATA_BUFFER_LEN, LiberalStorage, PostcardValue,
    RotaryButton, RotaryInput, ScaleRgb,
    ble_2::{Ble2, BleEvent},
    config::AUTO_CONNECT,
    liberal_renderer::render_display_2,
};

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

    let ws2812_gpio = p.GPIO7;
    let i2c_scl_gpio = p.GPIO5;
    let i2c_sda_gpio = p.GPIO6;
    let interrupt_gpio = p.GPIO1;
    let reset_gpio = p.GPIO8;

    let mut buffer = smart_led_buffer!(buffer_size_async(TOTAL_LEDS));
    let mut leds_adapter = SmartLedsAdapterAsync::new(
        Rmt::new(p.RMT, Rate::from_mhz(80))
            .unwrap()
            .into_async()
            .channel0,
        ws2812_gpio,
        &mut buffer,
    );
    leds_adapter
        .write(repeat(RGB8::default()).take(TOTAL_LEDS))
        .await
        .unwrap();

    // Scaling factor
    let aura_color = RGB8::new(255, 0, 255);
    let liberal_color = RGB8::new(0, 127, 255);
    let election_tracker_color = RGB8::new(0, 255, 0);

    let signal = Signal::<CriticalSectionRawMutex, _>::new();

    let i2c = Mutex::<CriticalSectionRawMutex, _>::new(
        I2c::new(p.I2C0, i2c::master::Config::default())
            .unwrap()
            .with_scl(i2c_scl_gpio)
            .with_sda(i2c_sda_gpio)
            .into_async(),
    );

    let mut mcp23017 = Mcp23017::new(
        I2cDeviceWithConfig::new(
            &i2c,
            i2c::master::Config::default().with_frequency(Rate::from_khz(400)),
        ),
        [false, false, false],
        BlockingAsync::new(Output::new(
            reset_gpio,
            Level::High,
            OutputConfig::default(),
        )),
        Input::new(interrupt_gpio, InputConfig::default().with_pull(Pull::Up)),
        Delay,
    );

    let mut flash = FlashStorage::new(p.FLASH);
    let mut pt_mem = [0; PARTITION_TABLE_MAX_LEN];
    let pt = read_partition_table(&mut flash, &mut pt_mem).unwrap();
    let nvs = pt
        .find_partition(PartitionType::Data(DataPartitionSubType::Nvs))
        .unwrap()
        .unwrap();
    let nvs_partition = nvs.as_embedded_storage(&mut flash);
    let map_config = MapConfig::new(0..nvs_partition.partition_size() as u32);
    let mut map_storage = MapStorage::<(), _, _>::new(
        BlockingAsync::new(nvs_partition),
        map_config,
        NoCache::new(),
    );
    let mut data_buffer = [Default::default(); LIBERAL_DATA_BUFFER_LEN];
    let mut stored_data = map_storage
        .fetch_item::<PostcardValue<LiberalStorage>>(&mut data_buffer, &())
        .await
        .unwrap()
        .unwrap_or_default();

    let mut game_state = GameState::new(if AUTO_CONNECT {
        stored_data.last_connected_peripheral.map(BdAddr::new)
    } else {
        None
    });
    let mut ble = Ble2::new();
    let controller = esp_radio::init().unwrap();
    let (ble_runner, mut ble) = ble.run(&controller, p.BT);
    let (gpio_expander_runner, expander_pins) = mcp23017.run();
    join4(
        render_display_2(&i2c, &signal),
        ble_runner,
        gpio_expander_runner,
        async {
            let mut rotary_input = RotaryInput::new(expander_pins.B2, expander_pins.B3).await;
            let mut rotary_button = RotaryButton::new(expander_pins.B1).await;

            signal.signal(game_state.clone());

            loop {
                use embassy_futures::select::{Either3::*, *};
                match select3(
                    rotary_input.next(),
                    rotary_button.wait_until_press(),
                    ble.next(),
                )
                .await
                {
                    First(direction) => {
                        info!("Direction: {}", direction);
                        game_state.process_input(match direction {
                            Direction::Clockwise => game_pure::Input::Down,
                            Direction::CounterClockwise => game_pure::Input::Up,
                        });
                    }
                    Second(()) => {
                        info!("Rotary button pressed");
                        game_state.process_input(game_pure::Input::Click);
                    }
                    Third(BleEvent::PeripheralScanned(address)) => {
                        info!("Address found: {}", address);
                        game_state.ble_peripheral_found(address.addr);
                    }
                    Third(BleEvent::ConnectionUpdate(state)) => match state {
                        ConnectState::Connected => {
                            info!("BLE connected");
                            game_state.ble_connected();
                        }
                        ConnectState::Connecting => {
                            info!("BLE disconnected");
                            game_state.ble_connected();
                        }
                    },
                }
                signal.signal(game_state.clone());
                match game_state.ble_action() {
                    BleAction::Scan => {
                        ble.scan();
                    }
                    BleAction::MaintainConnection(address) => {
                        ble.maintain_connection(Address {
                            kind: AddrKind::RANDOM,
                            addr: address,
                        });
                    }
                }
                {
                    let leds = game_state.get_leds();
                    let mut led_colors = [Default::default(); TOTAL_LEDS];
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
                        led_colors[election_tracker_led_index] =
                            election_tracker_color.scale(LED_BRIGHTNESS);
                    }
                    leds_adapter.write(led_colors).await.unwrap();
                }
            }
        },
    )
    .await;
}
