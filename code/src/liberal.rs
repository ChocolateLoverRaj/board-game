#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_futures::{join::*, select::*};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, signal::Signal,
};
use embassy_time::{Duration, Instant};
use esp_backtrace as _;
use esp_bootloader_esp_idf::partitions::{
    DataPartitionSubType, PARTITION_TABLE_MAX_LEN, PartitionType, read_partition_table,
};
use esp_hal::{
    efuse::Efuse,
    gpio::{Input, InputConfig, Level, Pull},
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
use sequential_storage::{
    cache::NoCache,
    map::{MapConfig, MapStorage},
};
use smart_leds::{RGB8, SmartLedsWriteAsync};
use trouble_host::prelude::*;

use lib::{
    CONNECTIONS_MAX, DATA_BUFFER_LEN, Debouncer, Direction, EmbeddedStorageAsyncWrapper,
    L2CAP_CHANNELS_MAX, LED_BRIGHTNESS, LiberalStorage, PostcardValue, RotaryInput, ScaleRgb,
    ScanningEventHandler,
    liberal_renderer::{ConnectingUiState, ScanningState, UiState, render_display},
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

    let signal = Signal::<CriticalSectionRawMutex, _>::new();
    join4(
        render_display(p.I2C0, i2c_scl_gpio, i2c_sda_gpio, &signal),
        async {},
        async {},
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
            let mut map_storage = MapStorage::<(), _, _>::new(
                EmbeddedStorageAsyncWrapper(nvs_partition),
                map_config,
                NoCache::new(),
            );
            let mut data_buffer = [Default::default(); DATA_BUFFER_LEN];
            let stored_data = map_storage
                .fetch_item::<PostcardValue<LiberalStorage>>(&mut data_buffer, &())
                .await
                .unwrap()
                .unwrap_or_default();

            let _trng_source = TrngSource::new(p.RNG, p.ADC1);
            let mut trng = Trng::try_new().unwrap();
            let radio = esp_radio::init().unwrap();
            let connector = BleConnector::new(&radio, p.BT, Default::default()).unwrap();
            let controller = ExternalController::<_, 20>::new(connector);

            // Using a fixed "random" address can be useful for testing. In real scenarios, one would
            // use e.g. the MAC 6 byte array as the address (how to get that varies by the platform).
            let our_address: Address = Address::random(Efuse::mac_address());
            info!("Our address = {:?}", our_address);

            let mut resources: HostResources<
                DefaultPacketPool,
                CONNECTIONS_MAX,
                L2CAP_CHANNELS_MAX,
            > = HostResources::new();
            let stack = trouble_host::new(controller, &mut resources)
                .set_random_address(our_address)
                .set_random_generator_seed(&mut trng)
                .set_io_capabilities(IoCapabilities::DisplayYesNo);

            for saved_bond_information in stored_data.saved_bonds.iter().cloned() {
                stack
                    .add_bond_information(saved_bond_information.into())
                    .unwrap();
            }

            let Host {
                mut central,
                mut runner,
                ..
            } = stack.build();

            // NOTE: Modify this to match the address of the peripheral you want to connect to.
            // Currently, it matches the address used by the peripheral examples
            // let target: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);

            if let Some(last_connected_peripheral) = &stored_data.last_connected_peripheral {
                let _ = join(runner.run(), async {
                    let address = BdAddr::new(*last_connected_peripheral);
                    let address = Address {
                        kind: AddrKind::RANDOM,
                        addr: address,
                    };
                    signal.signal(UiState::Connecting(ConnectingUiState {
                        address,
                        is_auto: true,
                    }));
                    let _connection = central
                        .connect(&ConnectConfig {
                            connect_params: Default::default(),
                            scan_config: ScanConfig {
                                filter_accept_list: &[(AddrKind::RANDOM, &address.addr)],
                                ..Default::default()
                            },
                        })
                        .await
                        .unwrap();
                    signal.signal(UiState::Connected(address));
                    core::future::pending::<()>().await;
                })
                .await;
            } else {
                let channel = Channel::new();
                let mut scanner = Scanner::new(central);
                let selected_address = match select(
                    runner.run_with_handler(&ScanningEventHandler { channel: &channel }),
                    async {
                        let mut scanning_state = ScanningState::default();
                        signal.signal(UiState::Scanning(scanning_state.clone()));
                        let _session = scanner
                            .scan(&ScanConfig {
                                active: true,
                                phys: PhySet::M1,
                                interval: Duration::from_secs(1),
                                window: Duration::from_secs(1),
                                ..Default::default()
                            })
                            .await
                            .unwrap();
                        let mut rotary_input = RotaryInput::new(rotary_dt_gpio, rotary_clk_gpio);
                        let mut partial_step_position = 0;
                        // 2 is naturally how the rotary encoder physically "snaps"
                        let steps_per_increment = 2;

                        let mut switch =
                            Input::new(rotary_sw_gpio, InputConfig::default().with_pull(Pull::Up));
                        let mut debouncer =
                            Debouncer::new(switch.level(), Duration::from_millis(1));

                        loop {
                            use Either3::*;
                            match select3(
                                rotary_input.next(),
                                channel.receive(),
                                select(switch.wait_for_any_edge(), debouncer.wait()),
                            )
                            .await
                            {
                                First(direction) => {
                                    info!("rotary direction: {}", direction);
                                    partial_step_position += match direction {
                                        Direction::Clockwise => 1,
                                        Direction::CounterClockwise => -1,
                                    };
                                    let selected_index_changed =
                                        if partial_step_position >= steps_per_increment {
                                            scanning_state.selected_index = scanning_state
                                                .selected_index
                                                .saturating_add(1)
                                                .min(1 + scanning_state.peripherals.len() - 1);
                                            true
                                        } else if partial_step_position <= -steps_per_increment {
                                            scanning_state.selected_index =
                                                scanning_state.selected_index.saturating_sub(1);
                                            true
                                        } else {
                                            false
                                        };
                                    if selected_index_changed {
                                        // TODO: Scroll into view
                                        partial_step_position = 0;
                                        signal.signal(UiState::Scanning(scanning_state.clone()));
                                    }
                                }
                                Second(address) => {
                                    // TODO: Maybe remove some peripherals if we haven't seen them for a while
                                    if !scanning_state.peripherals.contains(&address) {
                                        if scanning_state.peripherals.is_full() {
                                            scanning_state.peripherals.remove(0);
                                        }
                                        scanning_state.peripherals.push(address).unwrap();
                                        signal.signal(UiState::Scanning(scanning_state.clone()));
                                    }
                                }
                                Third(_) => {
                                    let level_changed =
                                        debouncer.process_data(switch.level(), Instant::now());
                                    if level_changed
                                        && debouncer.value() == Level::Low
                                        && scanning_state.selected_index > 0
                                    {
                                        break scanning_state.peripherals
                                            [scanning_state.selected_index - 1];
                                    }
                                }
                            }
                        }
                    },
                )
                .await
                {
                    Either::First(_) => unreachable!(),
                    Either::Second(selected_index) => selected_index,
                };
                info!("Connecting to {}", selected_address);
                signal.signal(UiState::Connecting(ConnectingUiState {
                    address: selected_address,
                    is_auto: false,
                }));
                let mut central = scanner.into_inner();
                let _ = join(runner.run(), async {
                    let _connection = central
                        .connect(&ConnectConfig {
                            connect_params: Default::default(),
                            scan_config: ScanConfig {
                                filter_accept_list: &[(AddrKind::RANDOM, &selected_address.addr)],
                                ..Default::default()
                            },
                        })
                        .await
                        .unwrap();
                    signal.signal(UiState::Connected(selected_address));
                    core::future::pending::<()>().await;
                })
                .await;

                // drop(session);
                // info!("Found a fascist board: {}. Done scanning.", address);
                // let mut central = scanner.into_inner();
                // let conn = central
                //     .connect(&ConnectConfig {
                //         connect_params: Default::default(),
                //         scan_config: ScanConfig {
                //             filter_accept_list: &[(address.kind, &address.addr)],
                //             ..Default::default()
                //         },
                //     })
                //     .await
                //     .unwrap();
                // // Only allow creating a new bond if we haven't connected to this peripheral before
                // let existing_bond_stored = stack
                //     .get_bond_information()
                //     .iter()
                //     .any(|bond| bond.identity == conn.peer_identity());
                // conn.set_bondable(!existing_bond_stored).unwrap();
                // conn.request_security().unwrap();
                // let bond = loop {
                //     let event = conn.next().await;
                //     info!("Connection event: {:#?}", event);
                //     match event {
                //         ConnectionEvent::Disconnected { reason } => {
                //             if existing_bond_stored
                //                 && reason == bt_hci::param::Status::AUTHENTICATION_FAILURE
                //             {
                //                 // warn!("Could not connect with existing bond. We can delete it and create a new bond.")
                //             } else {
                //                 panic!("BLE connection disconnected. reason: {:?}", reason);
                //             }
                //         }
                //         ConnectionEvent::PairingComplete {
                //             security_level: _,
                //             bond,
                //         } => {
                //             break bond;
                //         }
                //         ConnectionEvent::PassKeyDisplay(_) => {
                //             panic!("fascist board is DisplayOnly so unexpected PassKeyDisplay");
                //         }
                //         ConnectionEvent::PassKeyConfirm(_) => {
                //             panic!("fascist board is DisplayOnly so unexpected PassKeyConfirm");
                //         }
                //         ConnectionEvent::PassKeyInput => {
                //             panic!("this board is DisplayYesNo so unexpected PassKeyInput");
                //         }
                //         ConnectionEvent::PairingFailed(e) => {
                //             panic!("pairing failed: {e:?}");
                //         }
                //         _ => {
                //             panic!("unexpected connection event");
                //         }
                //     }
                // };
                // info!("bonded: {}", bond);
                // if !existing_bond_stored && let Some(bond) = bond {
                //     if stored_data.saved_bonds.is_full() {
                //         stored_data.saved_bonds.remove(0);
                //     }
                //     stored_data.saved_bonds.push(bond.into()).unwrap();
                //     map_storage
                //         .store_item(
                //             &mut [Default::default(); DATA_BUFFER_LEN],
                //             &(),
                //             &stored_data,
                //         )
                //         .await
                //         .unwrap();
                // }

                // info!("Connected, creating l2cap channel");
                // const PAYLOAD_LEN: usize = 27;
                // let config = L2capChannelConfig {
                //     mtu: Some(PAYLOAD_LEN as u16),
                //     ..Default::default()
                // };
                // let mut ch1 = L2capChannel::create(&stack, &conn, PSM_L2CAP_EXAMPLES, &config)
                //     .await
                //     .unwrap();
                // info!("New l2cap channel created, sending some data!");
                // for i in 0..10 {
                //     let tx = [i; PAYLOAD_LEN];
                //     ch1.send(&stack, &tx).await.unwrap();
                // }
                // info!("Sent data, waiting for them to be sent back");
                // let mut rx = [0; PAYLOAD_LEN];
                // for i in 0..10 {
                //     let len = ch1.receive(&stack, &mut rx).await.unwrap();
                //     assert_eq!(len, rx.len());
                //     assert_eq!(rx, [i; PAYLOAD_LEN]);
                // }

                // info!("Received successfully!");
            };
        },
    )
    .await;
}
