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
    Async, dma_circular_buffers_chunk_size,
    i2c::{self, master::I2c},
    i2s::{
        self,
        master::{Channels, DataFormat, I2s},
    },
    interrupt::software::SoftwareInterruptControl,
    peripherals::{GPIO5, GPIO6, GPIO10, GPIO20, GPIO21, I2C0, USB_DEVICE},
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

    let i2s = I2s::new(
        p.I2S0,
        p.DMA_CH0,
        i2s::master::Config::default()
            .with_sample_rate(Rate::from_khz(96))
            .with_data_format(DataFormat::Data32Channel32)
            .with_channels(Channels::MONO),
    )
    .unwrap()
    .into_async();
    spawner
        .spawn(speaker_task(i2s, p.GPIO10, p.GPIO20, p.GPIO21))
        .unwrap();

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

#[embassy_executor::task]
async fn speaker_task(
    i2s: I2s<'static, Async>,
    gpio10: GPIO10<'static>,
    gpio20: GPIO20<'static>,
    gpio21: GPIO21<'static>,
) {
    let (_rx_buffer, _rx_descriptors, tx_buffer, tx_descriptors) =
        dma_circular_buffers_chunk_size!(0, 4092 * 4, 4092);
    let tx = i2s
        .i2s_tx
        .with_bclk(gpio10)
        .with_dout(gpio20)
        .with_ws(gpio21)
        .build(tx_descriptors);
    let mut transfer = tx.write_dma_circular_async(tx_buffer).unwrap();
    let samples: [i32; _] = [
        0,
        52701887,
        105372028,
        157978697,
        210490206,
        262874923,
        315101294,
        367137860,
        418953276,
        470516330,
        521795963,
        572761285,
        623381597,
        673626408,
        723465451,
        772868706,
        821806413,
        870249095,
        918167571,
        965532978,
        1012316784,
        1058490807,
        1104027236,
        1148898640,
        1193077990,
        1236538675,
        1279254515,
        1321199780,
        1362349204,
        1402677999,
        1442161874,
        1480777044,
        1518500249,
        1555308767,
        1591180425,
        1626093615,
        1660027308,
        1692961061,
        1724875039,
        1755750016,
        1785567395,
        1814309215,
        1841958164,
        1868497585,
        1893911493,
        1918184580,
        1941302224,
        1963250500,
        1984016188,
        2003586778,
        2021950483,
        2039096240,
        2055013722,
        2069693341,
        2083126253,
        2095304369,
        2106220351,
        2115867625,
        2124240379,
        2131333571,
        2137142926,
        2141664947,
        2144896909,
        2146836865,
        2147483647,
        2146836865,
        2144896909,
        2141664947,
        2137142926,
        2131333571,
        2124240379,
        2115867625,
        2106220351,
        2095304369,
        2083126253,
        2069693341,
        2055013722,
        2039096240,
        2021950483,
        2003586778,
        1984016188,
        1963250500,
        1941302224,
        1918184580,
        1893911493,
        1868497585,
        1841958164,
        1814309215,
        1785567395,
        1755750016,
        1724875039,
        1692961061,
        1660027308,
        1626093615,
        1591180425,
        1555308767,
        1518500249,
        1480777044,
        1442161874,
        1402677999,
        1362349204,
        1321199780,
        1279254515,
        1236538675,
        1193077990,
        1148898640,
        1104027236,
        1058490807,
        1012316784,
        965532978,
        918167571,
        870249095,
        821806413,
        772868706,
        723465451,
        673626408,
        623381597,
        572761285,
        521795963,
        470516330,
        418953276,
        367137860,
        315101294,
        262874923,
        210490206,
        157978697,
        105372028,
        52701887,
        0,
        -52701887,
        -105372028,
        -157978697,
        -210490206,
        -262874923,
        -315101294,
        -367137860,
        -418953276,
        -470516330,
        -521795963,
        -572761285,
        -623381597,
        -673626408,
        -723465451,
        -772868706,
        -821806413,
        -870249095,
        -918167571,
        -965532978,
        -1012316784,
        -1058490807,
        -1104027236,
        -1148898640,
        -1193077990,
        -1236538675,
        -1279254515,
        -1321199780,
        -1362349204,
        -1402677999,
        -1442161874,
        -1480777044,
        -1518500249,
        -1555308767,
        -1591180425,
        -1626093615,
        -1660027308,
        -1692961061,
        -1724875039,
        -1755750016,
        -1785567395,
        -1814309215,
        -1841958164,
        -1868497585,
        -1893911493,
        -1918184580,
        -1941302224,
        -1963250500,
        -1984016188,
        -2003586778,
        -2021950483,
        -2039096240,
        -2055013722,
        -2069693341,
        -2083126253,
        -2095304369,
        -2106220351,
        -2115867625,
        -2124240379,
        -2131333571,
        -2137142926,
        -2141664947,
        -2144896909,
        -2146836865,
        -2147483647,
        -2146836865,
        -2144896909,
        -2141664947,
        -2137142926,
        -2131333571,
        -2124240379,
        -2115867625,
        -2106220351,
        -2095304369,
        -2083126253,
        -2069693341,
        -2055013722,
        -2039096240,
        -2021950483,
        -2003586778,
        -1984016188,
        -1963250500,
        -1941302224,
        -1918184580,
        -1893911493,
        -1868497585,
        -1841958164,
        -1814309215,
        -1785567395,
        -1755750016,
        -1724875039,
        -1692961061,
        -1660027308,
        -1626093615,
        -1591180425,
        -1555308767,
        -1518500249,
        -1480777044,
        -1442161874,
        -1402677999,
        -1362349204,
        -1321199780,
        -1279254515,
        -1236538675,
        -1193077990,
        -1148898640,
        -1104027236,
        -1058490807,
        -1012316784,
        -965532978,
        -918167571,
        -870249095,
        -821806413,
        -772868706,
        -723465451,
        -673626408,
        -623381597,
        -572761285,
        -521795963,
        -470516330,
        -418953276,
        -367137860,
        -315101294,
        -262874923,
        -210490206,
        -157978697,
        -105372028,
        -52701887,
    ]
    .map(|n| n / 8);
    // let mut transfer = tx.write_dma_circular(&samples).unwrap();
    let mut iterator = samples
        .iter()
        .cycle()
        .copied()
        .flat_map(|n| n.to_le_bytes());
    // loop {
    //     transfer
    //         .push_with(|buffer| {
    //             buffer.fill_with(|| iterator.next().unwrap());
    //             buffer.len()
    //         })
    //         .unwrap();
    //     yield_now().await;
    //     // let _ = transfer.push(samples.as_bytes());
    // }
    // transfer.
    loop {
        // transfer.push(samples.as_bytes()).await.unwrap();
        // for sample in samples {
        //     transfer.push(&sample.to_le_bytes()).await.unwrap();
        // }

        let bytes_pushed = transfer
            .push_with(|buffer| {
                buffer.fill_with(|| iterator.next().unwrap());
                buffer.len()
            })
            .await
            .unwrap();
        // info!("i2s bytes pushed: {}", bytes_pushed);
    }
    // let sample_rate = 44_100.0;
    // let step = 1.0 / sample_rate;
    // let frequency = 1_000.0;
    // let m = frequency / sample_rate * 2.0 * f64::consts::PI;
    // // let volume = 0.20;
    // let mut t = 0_f64;
    // loop {
    //     let sample = SINE_LUT[(t * m) as usize / SINE_LUT.len()] / 64;
    //     transfer.push(&sample.to_le_bytes()).await.unwrap();

    //     t += step;
    // }
}
