#![no_std]
#![no_main]

use core::{
    array,
    cell::RefCell,
    iter::{once, repeat, repeat_n, zip},
};

use collect_array_ext_trait::CollectArray;
use common::{Event, Request};
use defmt::{Debug2Format, debug, error, info, warn};
use display_interface::DisplayError;
use embassy_embedded_hal::{adapter::BlockingAsync, shared_bus::asynch::i2c::I2cDeviceWithConfig};
use embassy_executor::Spawner;
use embassy_futures::join::*;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, priority_channel, signal::Signal,
};
use embassy_time::{Delay, Timer};
use embedded_hal::digital::PinState;
use embedded_io_async::Write;
use esp_backtrace as _;
use esp_hal::{
    Async,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::{self, master::I2c},
    interrupt::software::SoftwareInterruptControl,
    peripherals::{GPIO7, GPIO8, RMT, UART0},
    rmt::Rmt,
    spi::{Mode, master::Spi},
    time::Rate,
    timer::timg::TimerGroup,
    uart::{self, Uart, UartRx, UartTx},
};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async, smart_led_buffer};
use heapless::Vec;
use lib::{
    RotaryButton, RotaryInput2,
    lazy_shared_spi::{LazySharedSpi, SpiDeviceWithConfig},
    lazy_shared_spi_2::{LazySharedSpi2, SpiDeviceWithConfig2},
};
use mcp23017_controller::Mcp23017;
use mfrc522::{
    AsyncMfrc522, AsyncPollingWaiterProvider, CardCommandError, ReqWupA, RxGain, Select,
    SpiRegisterAccess,
};
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

    // spawner.spawn(leds_task(p.RMT, p.GPIO7)).unwrap();

    // let mut reset_pin = Output::new(p.GPIO8, Level::High, OutputConfig::default());
    // reset_pin.set_low();
    // Timer::after_nanos(1000).await;
    // reset_pin.set_high();
    // Timer::after_nanos(37_740).await;

    let (uart_rx, uart_tx) = Uart::new(p.UART0, uart::Config::default().with_baudrate(4_500_000))
        .unwrap()
        .with_tx(p.GPIO8)
        .with_rx(p.GPIO7)
        .into_async()
        .split();
    spawner.spawn(uart_tx_task(uart_tx)).unwrap();
    spawner.spawn(uart_rx_task(uart_rx)).unwrap();

    // Soft reset
    info!("Soft resetting");
    REQUEST_SIGNALS[0].signal(Request::SoftReset);
    NEW_REQUEST_SIGNAL.signal(());
    SOFT_RESET_SIGNAL.wait().await;
    info!("Done  soft resetting");

    spawner.spawn(led_task()).unwrap();
    spawner.spawn(leds_task()).unwrap();
    spawner.spawn(rotary_switch_task()).unwrap();
    spawner.spawn(rotary_encoder_task()).unwrap();

    let i2c = Mutex::<CriticalSectionRawMutex, _>::new(
        I2c::new(p.I2C0, i2c::master::Config::default())
            .unwrap()
            .with_scl(p.GPIO5)
            .with_sda(p.GPIO6)
            .into_async(),
    );

    let result: Result<(), DisplayError> = async {
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
        display.init().await?;
        display.clear_buffer();
        display.flush().await?;
        let mut invert = false;
        loop {
            display.set_invert(invert).await?;
            Timer::after_millis(5000).await;
            invert = !invert;
        }
    }
    .await;
    if let Err(e) = result {
        warn!("display error: {}", Debug2Format(&e));
    }
    //let mut mcp23017 = Mcp23017::new(
    //    I2cDeviceWithConfig::new(
    //        &i2c,
    //        i2c::master::Config::default().with_frequency(Rate::from_khz(400)),
    //    ),
    //    [false, false, false],
    //    BlockingAsync::new(reset_pin),
    //    Input::new(p.GPIO1, InputConfig::default().with_pull(Pull::Up)),
    //    Delay,
    //);

    //let (mcp23017_runner, ep) = mcp23017.run();
    //let rotary_input = RotaryInput2::new();
    //let (rotary_input_runner, mut rotary_input) = rotary_input.run(ep.B2, ep.B3);

    // join(
    //     join5(
    //         mcp23017_runner,
    //         rotary_input_runner,
    //         async {
    //             loop {
    //                 info!("rotary position: {}", rotary_input.value());
    //                 rotary_input.watch().await;
    //             }
    //         },
    //         async {
    //             let result: Result<(), DisplayError> = async {
    //                 let i2c = I2cDeviceWithConfig::new(
    //                     &i2c,
    //                     i2c::master::Config::default().with_frequency(Rate::from_khz(400)),
    //                 );
    //                 let mut display = Ssd1306Async::new(
    //                     I2CDisplayInterface::new(i2c),
    //                     DisplaySize128x64,
    //                     DisplayRotation::Rotate0,
    //                 )
    //                 .into_buffered_graphics_mode();
    //                 display.init().await?;
    //                 display.clear_buffer();
    //                 display.flush().await?;
    //                 let mut invert = false;
    //                 loop {
    //                     display.set_invert(invert).await?;
    //                     Timer::after_millis(5000).await;
    //                     invert = !invert;
    //                 }
    //             }
    //             .await;
    //             if let Err(e) = result {
    //                 warn!("display error: {}", Debug2Format(&e));
    //             }
    //         },
    //         async {
    //             let mut rotary_button = RotaryButton::new(ep.B1).await;
    //             loop {
    //                 rotary_button.wait_until_press().await;
    //                 info!("rotary button pressed");
    //             }
    //         },
    //     ),
    //     async {
    //         let cs_pins = [ep.A0, ep.A1, ep.A2, ep.A3, ep.A4, ep.A5];
    //         let cs_pins =
    //             join_array(cs_pins.map(async |pin| pin.into_output(PinState::High).await)).await;
    //         let spi = LazySharedSpi2::<_, CriticalSectionRawMutex, _>::new(
    //             Spi::new(p.SPI2, Default::default())
    //                 .unwrap()
    //                 .with_sck(p.GPIO4)
    //                 .with_mosi(p.GPIO3)
    //                 .with_miso(p.GPIO2)
    //                 .into_async(),
    //             cs_pins,
    //         );
    //         let mut devices = array::from_fn::<_, 6, _>(|cs_index| {
    //             AsyncMfrc522::new(
    //                 SpiRegisterAccess::new(SpiDeviceWithConfig2::new(
    //                     &spi,
    //                     cs_index,
    //                     esp_hal::spi::master::Config::default()
    //                         .with_frequency(Rate::from_mhz(10))
    //                         .with_mode(Mode::_0),
    //                     Delay,
    //                 )),
    //                 AsyncPollingWaiterProvider::new(Delay, 25),
    //             )
    //         });
    //         let present_devices = join_array(devices.each_mut().map(async |device| {
    //             let version = device.version().await.unwrap();
    //             info!(
    //                 "version: {:#04X}. chip type: {:#04X}. version: {:#04X}",
    //                 version,
    //                 version.get_chip_type(),
    //                 version.get_version()
    //             );
    //             version.get_chip_type() == 0x9 && version.get_version() == 0x2
    //         }))
    //         .await;
    //         info!("present mfrc522 devices: {}", present_devices);
    //         let n_present_devices = present_devices
    //             .iter()
    //             .copied()
    //             .filter(|is_present| *is_present)
    //             .count();
    //         if n_present_devices > 0 {
    //             // Initialize all present devices
    //             for device in zip(&mut devices, present_devices)
    //                 .filter_map(|(device, is_present)| if is_present { Some(device) } else { None })
    //             {
    //                 device.soft_reset().await.unwrap();
    //                 device.init().await.unwrap();
    //                 device.set_antenna_gain(RxGain::DB18).await.unwrap();
    //             }
    //             // Check for cards at one device at a time
    //             // There are two reasons why we are only checking one device at a time
    //             // One is that they can interfere with each other
    //             // Another reason is to not overload the 5V to 3.3V converter on the esp32c3
    //             loop {
    //                 let mut ids = Vec::<_, 6>::new();
    //                 let mut i = 0;
    //                 while i < n_present_devices {
    //                     let (device, index) = zip(&mut devices, present_devices)
    //                         .filter(|(_device, is_present)| *is_present)
    //                         .nth(i)
    //                         .unwrap();
    //                     device.set_antenna_enabled(true).await.unwrap();
    //                     debug!("Doing  WUPA");
    //                     match device.card_command(ReqWupA::new(true)).await {
    //                         Ok(atq_a) => {
    //                             debug!("Doing SELECT");
    //                             match device.card_command(Select::new(&atq_a).unwrap()).await {
    //                                 Ok(uid) => {
    //                                     // info!("detected uid: {}", uid);
    //                                     ids.push(uid).unwrap();
    //                                 }
    //                                 Err(CardCommandError::CardCommand(e)) => {
    //                                     warn!("SELECT error: {}", e);
    //                                 }
    //                                 result => {
    //                                     result.unwrap();
    //                                 }
    //                             }
    //                         }
    //                         Err(CardCommandError::CardCommand(e)) => {
    //                             debug!("WupA error: {}", e);
    //                         }
    //                         result => {
    //                             result.unwrap();
    //                         }
    //                     };
    //                     device.set_antenna_enabled(false).await.unwrap();
    //                     i += 1;
    //                 }
    //                 info!("scanned ids: {}", ids);
    //                 Timer::after_millis(500).await;
    //             }
    //         }
    //     },
    // )
    // .await;
}

type M = CriticalSectionRawMutex;

static REQUEST_SIGNALS: [Signal<M, Request>; 5] = [
    Signal::new(),
    Signal::new(),
    Signal::new(),
    Signal::new(),
    Signal::new(),
];
static NEW_REQUEST_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

#[embassy_executor::task]
async fn uart_tx_task(mut uart_tx: UartTx<'static, Async>) {
    let mut buffer = [Default::default(); 1024];
    loop {
        NEW_REQUEST_SIGNAL.wait().await;
        for request in REQUEST_SIGNALS.iter().flat_map(|signal| signal.try_take()) {
            let bytes_written = postcard::to_slice_cobs(&request, &mut buffer)
                .unwrap()
                .len();
            match uart_tx.write_all(&buffer[..bytes_written]).await {
                Ok(()) => {}
                Err(e) => {
                    warn!("Error writing to UART: {}", e);
                }
            }
        }
    }
}

static SOFT_RESET_SIGNAL: Signal<M, ()> = Signal::new();
static ROTARY_SWITCH_SIGNAL: Signal<M, bool> = Signal::new();
static ROTARY_ENCODER_SIGNAL: Signal<M, i64> = Signal::new();

#[embassy_executor::task]
async fn uart_rx_task(mut uart_rx: UartRx<'static, Async>) {
    let mut buffer = [Default::default(); 1024];
    let mut buffer_len = 0;
    loop {
        match uart_rx.read_async(&mut buffer[buffer_len..]).await {
            Ok(bytes_read) => {
                buffer_len += bytes_read;
                loop {
                    let data = &mut buffer[..buffer_len];
                    let zero_pos = match data.iter().copied().position(|byte| byte == 0) {
                        Some(pos) => pos,
                        None => break,
                    };
                    let packet_len = zero_pos + 1;
                    match postcard::from_bytes_cobs::<Event>(&mut data[..packet_len]) {
                        Ok(event) => match event {
                            Event::SoftResetComplete => {
                                SOFT_RESET_SIGNAL.signal(());
                            }
                            Event::RotarySwitch(value) => {
                                ROTARY_SWITCH_SIGNAL.signal(value);
                            }
                            Event::RotaryEncoder(value) => {
                                ROTARY_ENCODER_SIGNAL.signal(value);
                            }
                        },
                        Err(e) => {
                            error!("error deserializing packet: {}", e);
                        }
                    }
                    buffer.copy_within(packet_len..buffer_len, 0);
                    buffer_len -= packet_len;
                }
            }
            Err(e) => {
                error!("Error receiving UART data: {}", e);
            }
        }
    }
}

#[embassy_executor::task]
async fn leds_task() {
    let mut n = 0;
    const TOTAL_LEDS: usize = 64;
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
        let leds = brightness(
            repeat_n(alternate_color, n)
                .chain(once(RGB::new(255, 0, 0)))
                .chain(repeat_n(alternate_color, TOTAL_LEDS - n - 1)),
            5,
        );

        REQUEST_SIGNALS[2].signal(Request::SetLeds(leds.collect_array().unwrap()));
        NEW_REQUEST_SIGNAL.signal(());
        Timer::after_millis(100).await;
    }
}

#[embassy_executor::task]
async fn led_task() {
    let mut led_level = false;
    loop {
        REQUEST_SIGNALS[1].signal(Request::SetLed(led_level));
        NEW_REQUEST_SIGNAL.signal(());
        led_level = !led_level;
        Timer::after_secs(1).await;
    }
}

#[embassy_executor::task]
async fn rotary_switch_task() {
    REQUEST_SIGNALS[3].signal(Request::WatchRotarySwitch(true));
    NEW_REQUEST_SIGNAL.signal(());
    loop {
        let is_pressed = ROTARY_SWITCH_SIGNAL.wait().await;
        info!("rotary button pressed? {}", is_pressed);
    }
}

#[embassy_executor::task]
async fn rotary_encoder_task() {
    REQUEST_SIGNALS[4].signal(Request::WatchRotaryEncoder(true));
    NEW_REQUEST_SIGNAL.signal(());
    loop {
        let position = ROTARY_ENCODER_SIGNAL.wait().await;
        info!("rotary encoder position: {}", position);
    }
}
