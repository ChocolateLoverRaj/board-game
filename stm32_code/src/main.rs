#![no_std]
#![no_main]
mod debouncer;

use core::array;

use crate::debouncer::Debouncer;
use common::{Event, MAX_NFC_READERS, Request};
use defmt::{Debug2Format, debug, info, warn};
use embassy_embedded_hal::shared_bus::asynch::spi::SpiDeviceWithConfig;
use embassy_executor::Spawner;
use embassy_futures::select::{Either3, Either5, select3, select5};
use embassy_stm32::{
    Config, Peri, bind_interrupts,
    exti::ExtiInput,
    gpio::{AnyPin, Level, Output, Pull, Speed},
    mode::Async,
    peripherals::{
        DMA1_CH3, DMA1_CH4, DMA1_CH5, EXTI0, EXTI1, EXTI2, EXTI8, EXTI9, EXTI10, PA0, PA1, PA2,
        PA7, PA8, PA9, PA10, PB13, PB14, PB15, SPI1, SPI2,
    },
    rcc::{self, APBPrescaler, Hse, HseMode, Pll, PllMul, PllPreDiv, PllSource, Sysclk},
    spi::{self, Spi},
    time::{hz, khz, mhz},
    usart::{Uart, UartTx},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Delay, Duration, Instant, Timer, WithTimeout};
use embedded_io_async::Write;
use heapless::{Vec, index_set::FnvIndexSet};
use hex_fmt::HexFmt;
use mfrc522::{
    AsyncMfrc522, AsyncPollingWaiterProvider, CardCommandError, Mfrc522, ReqWupA, RxGain, Select,
    SpiRegisterAccess,
};
use pure_rotary_encoder::{Direction, RotaryEncoder, RotaryPinsState};
use smart_leds::{RGB, SmartLedsWriteAsync};
use ws2812_async::{Grb, Ws2812};

use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USART2 => embassy_stm32::usart::InterruptHandler<embassy_stm32::peripherals::USART2>;
    EXTI9_5 => embassy_stm32::exti::InterruptHandler<embassy_stm32::interrupt::typelevel::EXTI9_5>;
    EXTI15_10 => embassy_stm32::exti::InterruptHandler<embassy_stm32::interrupt::typelevel::EXTI15_10>;
});

type M = CriticalSectionRawMutex;

static NEW_EVENT_SIGNAL: Signal<M, ()> = Signal::new();
static EVENT_SIGNALS: [Signal<M, Event>; 4] =
    [Signal::new(), Signal::new(), Signal::new(), Signal::new()];

#[embassy_executor::main]
async fn main(spawner: Spawner) -> ! {
    let p = embassy_stm32::init({
        let mut config = Config::default();
        config.rcc = {
            let mut rcc = rcc::Config::new();
            rcc.hse = Some(Hse {
                freq: mhz(8),
                mode: HseMode::Oscillator,
            });
            rcc.pll = Some(Pll {
                prediv: PllPreDiv::DIV1,
                mul: PllMul::MUL9,
                src: PllSource::HSE,
            });
            rcc.apb1_pre = APBPrescaler::DIV2;
            rcc.apb2_pre = APBPrescaler::DIV1;
            rcc.sys = Sysclk::PLL1_P;
            rcc
        };
        config
    });

    spawner.spawn(leds_task(p.SPI1, p.PA7, p.DMA1_CH3)).unwrap();
    spawner.spawn(rotary_switch_task(p.PA10, p.EXTI10)).unwrap();
    spawner
        .spawn(rotary_encoder_task(p.PA9, p.EXTI9, p.PA8, p.EXTI8))
        .unwrap();

    let mut reset_pin = Output::new(p.PB11, Level::High, Speed::Low);
    // reset_pin.set_low();
    // Timer::after_nanos(1000).await;
    // reset_pin.set_high();
    // Timer::after_nanos(37_740).await;
    // Timer::after_secs(1).await;

    spawner
        .spawn(nfc_task(
            p.SPI2,
            p.PB15,
            p.PB14,
            p.PB13,
            p.DMA1_CH5,
            p.DMA1_CH4,
            {
                let mut v = Vec::new();
                v.push(p.PA0.into()).ok().unwrap();
                v.push(p.PA4.into()).ok().unwrap();
                v.push(p.PB6.into()).ok().unwrap();
                v.push(p.PB0.into()).ok().unwrap();
                v.push(p.PB1.into()).ok().unwrap();
                v.push(p.PB5.into()).ok().unwrap();
                v
            },
        ))
        .unwrap();

    let mut led = Output::new(p.PC13, Level::High, Speed::Low);

    let uart = Uart::new(p.USART2, p.PA3, p.PA2, Irqs, p.DMA1_CH7, p.DMA1_CH6, {
        let mut config = embassy_stm32::usart::Config::default();
        config.baudrate = 2_250_000;
        config.eager_reads = Some(1);
        config
    })
    .unwrap();
    let (uart_tx, uart_rx) = uart.split();
    spawner.spawn(uart_tx_task(uart_tx)).unwrap();

    let mut dma_buf = [Default::default(); 1024];
    let mut uart_rx = uart_rx.into_ring_buffered(&mut dma_buf);
    let mut buffer = [Default::default(); 1024];
    let mut buffer_bytes = 0;
    loop {
        debug!("waiting to read bytes");
        let new_bytes_read = match uart_rx.read(&mut buffer[buffer_bytes..]).await {
            Ok(n) => n,
            Err(e) => {
                warn!("error reading UART: {}", e);
                continue;
            }
        };
        {
            let new_bytes = &buffer[buffer_bytes..buffer_bytes + new_bytes_read];
            debug!("received bytes: {}", new_bytes);
            buffer_bytes += new_bytes_read;
        }
        loop {
            let bytes = &mut buffer[..buffer_bytes];
            let zero_index = match bytes.iter().copied().position(|byte| byte == 0) {
                Some(zero_index) => zero_index,
                None => break,
            };
            let packet_len = zero_index + 1;
            match postcard::from_bytes_cobs::<Request>(&mut buffer[..packet_len]) {
                Ok(request) => match request {
                    Request::SoftReset => {
                        led.set_high();
                        LEDS_SIGNAL.signal([Default::default(); _]);
                        WATCH_ROTARY_SWITCH_SIGNAL.signal(false);
                        WATCH_ROTARY_ENCODER_SIGNAL.signal(false);
                        WATCH_NFC_SIGNAL.signal(false);
                        EVENT_SIGNALS[0].signal(Event::SoftResetComplete);
                        NEW_EVENT_SIGNAL.signal(());
                    }
                    Request::SetLed(state) => {
                        led.set_level(state.into());
                    }
                    Request::SetLeds(colors) => {
                        LEDS_SIGNAL.signal(colors);
                    }
                    Request::WatchRotarySwitch(watch) => {
                        WATCH_ROTARY_SWITCH_SIGNAL.signal(watch);
                    }
                    Request::WatchRotaryEncoder(watch) => {
                        WATCH_ROTARY_ENCODER_SIGNAL.signal(watch);
                    }
                    Request::WatchNfc(watch) => {
                        WATCH_NFC_SIGNAL.signal(watch);
                    }
                },
                Err(e) => {
                    warn!("Error: {}", e);
                }
            }
            buffer.copy_within(packet_len..buffer_bytes, 0);
            buffer_bytes -= packet_len;
        }
    }
}

#[embassy_executor::task]
async fn uart_tx_task(mut uart_tx: UartTx<'static, Async>) {
    let mut buffer = [Default::default(); 1024];
    loop {
        NEW_EVENT_SIGNAL.wait().await;
        for event in EVENT_SIGNALS.iter().flat_map(|event| event.try_take()) {
            let bytes_written = postcard::to_slice_cobs(&event, &mut buffer).unwrap().len();
            match uart_tx.write_all(&buffer[..bytes_written]).await {
                Ok(()) => {}
                Err(e) => {
                    warn!("Error writing to UART: {}", e);
                }
            }
        }
    }
}

const TOTAL_LEDS: usize = 64;
static LEDS_SIGNAL: Signal<CriticalSectionRawMutex, [RGB<u8>; TOTAL_LEDS]> = Signal::new();
#[embassy_executor::task]
async fn leds_task(
    spi: Peri<'static, SPI1>,
    pin: Peri<'static, PA7>,
    dma: Peri<'static, DMA1_CH3>,
) {
    let spi = Spi::new_txonly_nosck(spi, pin, dma, {
        let mut config = spi::Config::default();
        config.frequency = khz(3800);
        config
    });
    let mut leds = Ws2812::<_, Grb, TOTAL_LEDS>::new(spi);
    loop {
        let colors = LEDS_SIGNAL.wait().await;
        leds.write(colors).await.unwrap();
    }
}

static WATCH_ROTARY_SWITCH_SIGNAL: Signal<M, bool> = Signal::new();
#[embassy_executor::task]
async fn rotary_switch_task(pin: Peri<'static, PA10>, exti: Peri<'static, EXTI10>) {
    let mut sw = ExtiInput::new(pin, exti, Pull::Up, Irqs);
    loop {
        // Wait for enable
        loop {
            if WATCH_ROTARY_SWITCH_SIGNAL.wait().await {
                break;
            }
        }
        let mut debouncer = Debouncer::new(Duration::from_millis(1));
        loop {
            let new_value = debouncer.process_data(sw.get_level(), Instant::now());
            if let Some(&new_value) = new_value {
                EVENT_SIGNALS[1].signal(Event::RotarySwitch(new_value == Level::Low));
                NEW_EVENT_SIGNAL.signal(());
            }
            match select3(
                {
                    let value = *debouncer.maybe_stable_value().unwrap();
                    let sw = &mut sw;
                    async move {
                        match value {
                            Level::Low => sw.wait_for_high().await,
                            Level::High => sw.wait_for_low().await,
                        }
                    }
                },
                debouncer.wait(),
                async {
                    loop {
                        if !WATCH_ROTARY_SWITCH_SIGNAL.wait().await {
                            break;
                        }
                    }
                },
            )
            .await
            {
                Either3::First(()) | Either3::Second(()) => {}
                Either3::Third(()) => {
                    break;
                }
            }
        }
    }
}

static WATCH_ROTARY_ENCODER_SIGNAL: Signal<M, bool> = Signal::new();
#[embassy_executor::task]
async fn rotary_encoder_task(
    dt: Peri<'static, PA9>,
    dt_exti: Peri<'static, EXTI9>,
    clk: Peri<'static, PA8>,
    clk_exti: Peri<'static, EXTI8>,
) {
    let mut dt = ExtiInput::new(dt, dt_exti, Pull::Up, Irqs);
    let mut clk = ExtiInput::new(clk, clk_exti, Pull::Up, Irqs);
    loop {
        // Wait for enable
        loop {
            if WATCH_ROTARY_ENCODER_SIGNAL.wait().await {
                break;
            }
        }
        let mut dt_debouncer = Debouncer::new(Duration::from_millis(1));
        let mut clk_debouncer = Debouncer::new(Duration::from_millis(1));
        let mut rotary_encoder = None;
        let mut position = 0;
        loop {
            let new_dt = dt_debouncer.process_data(dt.get_level(), Instant::now());
            let new_clk = clk_debouncer.process_data(clk.get_level(), Instant::now());
            let state_changed = new_dt.is_some() || new_clk.is_some();
            if state_changed
                && let Some((dt, clk)) = dt_debouncer
                    .stable_value()
                    .and_then(|dt| clk_debouncer.stable_value().map(|clk| (*dt, *clk)))
            {
                let pins_state = RotaryPinsState {
                    dt: dt == Level::Low,
                    clk: clk == Level::Low,
                };
                if let Some(direction) = rotary_encoder
                    .get_or_insert(RotaryEncoder::new(pins_state))
                    .process_data(pins_state)
                {
                    position += match direction {
                        Direction::Clockwise => 1,
                        Direction::CounterClockwise => -1,
                    };
                    EVENT_SIGNALS[2].signal(Event::RotaryEncoder(position));
                    NEW_EVENT_SIGNAL.signal(());
                }
            }
            match select5(
                {
                    let value = *dt_debouncer.maybe_stable_value().unwrap();
                    let dt = &mut dt;
                    async move {
                        match value {
                            Level::Low => dt.wait_for_high().await,
                            Level::High => dt.wait_for_low().await,
                        }
                    }
                },
                dt_debouncer.wait(),
                {
                    let value = *clk_debouncer.maybe_stable_value().unwrap();
                    let clk = &mut clk;
                    async move {
                        match value {
                            Level::Low => clk.wait_for_high().await,
                            Level::High => clk.wait_for_low().await,
                        }
                    }
                },
                clk_debouncer.wait(),
                async {
                    loop {
                        if !WATCH_ROTARY_ENCODER_SIGNAL.wait().await {
                            break;
                        }
                    }
                },
            )
            .await
            {
                Either5::First(())
                | Either5::Second(())
                | Either5::Third(())
                | Either5::Fourth(()) => {}
                Either5::Fifth(()) => {
                    break;
                }
            }
        }
    }
}

static WATCH_NFC_SIGNAL: Signal<M, bool> = Signal::new();
#[embassy_executor::task]
async fn nfc_task(
    spi: Peri<'static, SPI2>,
    pico: Peri<'static, PB15>,
    poci: Peri<'static, PB14>,
    sck: Peri<'static, PB13>,
    tx_dma: Peri<'static, DMA1_CH5>,
    rx_dma: Peri<'static, DMA1_CH4>,
    cs_pins: Vec<Peri<'static, AnyPin>, MAX_NFC_READERS>,
) {
    let spi = Mutex::<M, _>::new(Spi::new(
        spi,
        sck,
        pico,
        poci,
        tx_dma,
        rx_dma,
        spi::Config::default(),
    ));
    let mut nfc_readers = {
        let mut nfc_readers = cs_pins
            .into_iter()
            .map(|cs_pin| {
                AsyncMfrc522::new(
                    SpiRegisterAccess::new(SpiDeviceWithConfig::new(
                        &spi,
                        Output::new(cs_pin, Level::High, Speed::Low),
                        {
                            let mut config = spi::Config::default();
                            config.frequency = hz(10_000_000);
                            config.mode = spi::MODE_0;
                            config
                        },
                    )),
                    AsyncPollingWaiterProvider::new(Delay, 25),
                )
            })
            .collect::<Vec<_, MAX_NFC_READERS>>();
        // let mut last_logged = None;
        // loop {
        //     let now = Instant::now();
        //     let should_log = match last_logged {
        //         Some(last_logged) => (now - last_logged) >= Duration::from_secs(1),
        //         None => true,
        //     };
        //     if should_log {
        //         last_logged = Some(now);
        //     }
        //     for (i, nfc_reader) in nfc_readers.iter_mut().enumerate() {
        //         for _ in 0..1 {
        //             if let Ok(version) = nfc_reader.version().await {
        //                 if [0x8, 0x9].contains(&version.get_chip_type())
        //                     && version.get_version() == 0x2
        //                 {
        //                     if should_log {
        //                         info!("[{}] NFC reader good", i);
        //                     }
        //                 } else {
        //                     if should_log {
        //                         warn!("[{}] buggy NFC reader: {:#04X}", i, version);
        //                     }
        //                 }
        //             } else {
        //                 warn!("[{}] NFC reader error", i);
        //             }
        //         }
        //     }
        //     // Timer::after_secs(1).await;
        // }
        let mut working_nfc_readers = 0;
        for (i, nfc_reader) in nfc_readers.iter_mut().enumerate() {
            let version = async {
                nfc_reader
                    .soft_reset()
                    .with_timeout(Duration::from_secs(1))
                    .await
                    .ok()
                    .and_then(|result| result.ok())?;
                nfc_reader.init().await.ok()?;
                nfc_reader.set_antenna_gain(RxGain::DB18).await.ok()?;
                let version = nfc_reader.version().await.ok()?;
                info!(
                    "[{}] NFC reader chip type: {:#04X}, version: {:#04X}",
                    i,
                    version.get_chip_type(),
                    version.get_version()
                );
                if [0x8, 0x9].contains(&version.get_chip_type()) && version.get_version() == 0x2 {
                    Some(version)
                } else {
                    warn!("[{}] buggy NFC reader", i);
                    None
                }
            }
            .await;
            if version.is_some() {
                working_nfc_readers += 1;
            } else {
                break;
            }
        }
        info!(
            "{}/{} working NFC readers",
            working_nfc_readers, MAX_NFC_READERS
        );
        nfc_readers.drain(working_nfc_readers..);
        nfc_readers
    };

    // Check for cards at one device at a time
    // There are two reasons why we are only checking one device at a time
    // One is that they can interfere with each other
    // Another reason is to not overload the 5V to 3.3V converter on the esp32c3
    let mut enabled = false;
    loop {
        if let Some(new_enabled) = WATCH_NFC_SIGNAL.try_take() {
            enabled = new_enabled;
        }
        if !enabled {
            enabled = WATCH_NFC_SIGNAL.wait().await;
            continue;
        }

        // let mut ids = FnvIndexSet::<_, { MAX_NFC_READERS.next_power_of_two() }>::new();
        // let mut detected_ids = array::from_fn::<_, MAX_NFC_READERS, _>(|_| None);
        let mut detected_ids = Vec::<_, MAX_NFC_READERS>::new();
        // let before = Instant::now();
        for (_i, device) in nfc_readers.iter_mut().enumerate() {
            // let version = device.version().await.unwrap();
            // if [0x8, 0x9].contains(&version.get_chip_type()) && version.get_version() == 0x2 {
            //     info!("[{}] version good", i);
            // } else {
            //     info!(
            //         "[{}] NFC reader chip type: {:#04X}, version: {:#04X}",
            //         i,
            //         version.get_chip_type(),
            //         version.get_version()
            //     );
            // }
            // Timer::after_millis(100).await;
            device.set_antenna_enabled(true).await.unwrap();
            debug!("Doing  WUPA");
            let uid = match device.card_command(ReqWupA::new(true)).await {
                Ok(atq_a) => {
                    if let Ok(select) = Select::new(&atq_a) {
                        match device.card_command(select).await {
                            Ok(uid) => {
                                // info!("detected uid: {}", uid);
                                // ids.insert(uid).unwrap();
                                Some(uid)
                            }
                            Err(CardCommandError::CardCommand(e)) => {
                                debug!("SELECT error: {}", e);
                                None
                            }
                            Err(_e) => {
                                debug!("SELECT error");
                                None
                            }
                        }
                    } else {
                        None
                    }
                }
                Err(CardCommandError::CardCommand(e)) => {
                    debug!("WupA error: {}", e);
                    None
                }
                Err(_e) => {
                    debug!("WUPA error");
                    None
                }
            };
            detected_ids.push(uid).unwrap();
            device.set_antenna_enabled(false).await.unwrap();
        }
        // let ids_hex = detected_ids
        //     .iter()
        //     .map(|id| id.as_ref().map(|id| HexFmt(id.as_bytes())))
        //     .collect::<Vec<_, MAX_NFC_READERS>>();
        // info!(
        //     "scanned ids: {:#?} in {}us",
        //     Debug2Format(&ids_hex),
        //     before.elapsed().as_micros()
        // );
        EVENT_SIGNALS[3].signal(Event::Nfc(detected_ids));
        NEW_EVENT_SIGNAL.signal(());

        if nfc_readers.is_empty() {
            // TODO: Only send this once
            Timer::after_secs(1).await;
        }
        // Timer::after_millis(25).await;
    }
}
