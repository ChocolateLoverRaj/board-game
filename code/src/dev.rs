#![no_std]
#![no_main]

use core::{
    array,
    iter::{once, repeat_n},
};

use collect_array_ext_trait::CollectArray;
use common::{Event, MAX_NFC_READERS, Request};
use defmt::{Debug2Format, debug, error, info, warn};
use display_interface::DisplayError;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDeviceWithConfig;
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal, watch::Watch,
};
use embassy_time::{Duration, Instant, Timer};
use embedded_io_async::{Read, Write};
use esp_backtrace as _;
use esp_hal::{
    Async,
    i2c::{self, master::I2c},
    interrupt::software::SoftwareInterruptControl,
    peripherals::{GPIO5, GPIO6, I2C0, USB_DEVICE},
    time::Rate,
    timer::timg::TimerGroup,
    uart::{self, Uart, UartRx, UartTx},
    usb_serial_jtag::UsbSerialJtag,
};
use heapless::Vec;
use mfrc522::Uid;
use smart_leds::{RGB, brightness};
use ssd1306::{I2CDisplayInterface, Ssd1306Async, prelude::*, size::DisplaySize128x64};

esp_bootloader_esp_idf::esp_app_desc!();

static IS_RUNNING: Watch<M, bool, 3> = Watch::new_with(true);

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

    spawner.spawn(usb_task(p.USB_DEVICE)).unwrap();
    spawner
        .spawn(display_task(p.I2C0, p.GPIO5, p.GPIO6))
        .unwrap();
    let (uart_rx, uart_tx) = Uart::new(p.UART0, uart::Config::default().with_baudrate(2_250_000))
        .unwrap()
        .with_tx(p.GPIO0)
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
    spawner.spawn(nfc_task()).unwrap();
}

#[embassy_executor::task]
async fn usb_task(usb_device: USB_DEVICE<'static>) {
    let mut jtag = UsbSerialJtag::new(usb_device).into_async();
    let mut buffer = [Default::default(); 1024];
    let mut is_running = true;
    loop {
        let bytes_read = jtag.read(&mut buffer).await.unwrap();
        for byte in &buffer[..bytes_read] {
            match byte {
                b'p' => {
                    if is_running {
                        info!("pausing");
                    } else {
                        info!("resuming");
                    }
                    is_running = !is_running;
                    IS_RUNNING.sender().send(is_running);
                }
                _ => {}
            }
        }
    }
}

#[embassy_executor::task]
async fn display_task(i2c: I2C0<'static>, scl: GPIO5<'static>, sda: GPIO6<'static>) {
    let i2c = Mutex::<CriticalSectionRawMutex, _>::new(
        I2c::new(i2c, i2c::master::Config::default())
            .unwrap()
            .with_scl(scl)
            .with_sda(sda)
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
        let mut receiver = IS_RUNNING.receiver().unwrap();
        loop {
            loop {
                if receiver.get().await {
                    break;
                }
                receiver.changed().await;
            }
            match select(
                async {
                    let mut invert = false;
                    loop {
                        display.set_invert(invert).await?;
                        Timer::after_millis(5000).await;
                        invert = !invert;
                    }
                },
                async {
                    loop {
                        if !receiver.get().await {
                            break;
                        }
                        receiver.changed().await;
                    }
                },
            )
            .await
            {
                Either::First(result) => result,
                Either::Second(()) => Ok(()),
            }?;
            display.set_invert(false).await?;
        }
    }
    .await;
    if let Err(e) = result {
        warn!("display error: {}", Debug2Format(&e));
    }
}

type M = CriticalSectionRawMutex;

static REQUEST_SIGNALS: [Signal<M, Request>; 6] = [
    Signal::new(),
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
static NFC_SIGNAL: Signal<M, Vec<Option<Uid>, MAX_NFC_READERS>> = Signal::new();

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
                            Event::Nfc(value) => {
                                NFC_SIGNAL.signal(value);
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
                debug!("Error receiving UART data: {}", e);
            }
        }
    }
}

#[embassy_executor::task]
async fn leds_task() {
    const TOTAL_LEDS: usize = 64;
    // We can push this to the limit with an interval of 3ms!
    // But to actually be able to see that it's not skipping any LEDs, we reduce this.
    let frame_interval = Duration::from_millis(100);
    let start_time = Instant::now();
    let mut last_rendered_frame = None;
    let mut receiver = IS_RUNNING.receiver().unwrap();
    loop {
        if !receiver.try_get().unwrap() {
            REQUEST_SIGNALS[2].signal(Request::SetLeds(array::repeat(Default::default())));
            NEW_REQUEST_SIGNAL.signal(());
            receiver.changed_and(|bool| *bool).await;
        }
        if let Some(frame_number) = last_rendered_frame {
            Timer::at(start_time + frame_interval * (frame_number as u32 + 1)).await;
        }
        let frame_number = start_time.elapsed().as_ticks() / frame_interval.as_ticks();
        last_rendered_frame = Some(frame_number);
        let n = frame_number as usize % TOTAL_LEDS;
        // let alternate_color = if n.is_multiple_of(2) {
        //     RGB::new(255, 165, 0)
        // } else {
        //     RGB::new(255, 50, 0)
        // };
        let alternate_color = Default::default();
        let leds = brightness(
            repeat_n(alternate_color, n)
                .chain(once(RGB::new(255, 0, 0)))
                .chain(repeat_n(alternate_color, TOTAL_LEDS - n - 1)),
            5,
        );

        REQUEST_SIGNALS[2].signal(Request::SetLeds(leds.collect_array().unwrap()));
        NEW_REQUEST_SIGNAL.signal(());
    }
}

#[embassy_executor::task]
async fn led_task() {
    let mut led_level = false;
    let mut receiver = IS_RUNNING.receiver().unwrap();
    loop {
        if !receiver.try_get().unwrap() {
            REQUEST_SIGNALS[1].signal(Request::SetLed(true));
            NEW_REQUEST_SIGNAL.signal(());
            receiver.changed_and(|bool| *bool).await;
        }
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

#[embassy_executor::task]
async fn nfc_task() {
    REQUEST_SIGNALS[5].signal(Request::WatchNfc(true));
    NEW_REQUEST_SIGNAL.signal(());
    let mut last_updated = None;
    loop {
        let nfc_tags = NFC_SIGNAL.wait().await;
        let now = Instant::now();
        let previously_updated = last_updated.replace(now);
        info!(
            "NFC tags: {} in {}us",
            nfc_tags,
            previously_updated.map(|before| (now - before).as_micros())
        );
    }
}
