#![no_std]
#![no_main]
mod debouncer;

use crate::debouncer::Debouncer;
use common::{Event, Request};
use defmt::{debug, warn};
use embassy_executor::Spawner;
use embassy_futures::select::{Either3, Either5, select3, select5};
use embassy_stm32::{
    Config, Peri, bind_interrupts,
    exti::ExtiInput,
    gpio::{Level, Output, Pull, Speed},
    mode::Async,
    peripherals::{DMA1_CH3, EXTI0, EXTI1, EXTI2, PA0, PA1, PA2, PA7, SPI1},
    rcc::{self, APBPrescaler, Hse, HseMode, Pll, PllMul, PllPreDiv, PllSource, Sysclk},
    spi::{self, Spi},
    time::{khz, mhz},
    usart::{Uart, UartTx},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant};
use embedded_io_async::Write;
use pure_rotary_encoder::{Direction, RotaryEncoder, RotaryPinsState};
use smart_leds::{RGB, SmartLedsWriteAsync};
use ws2812_async::{Grb, Ws2812};

use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USART1 => embassy_stm32::usart::InterruptHandler<embassy_stm32::peripherals::USART1>;
    EXTI0 => embassy_stm32::exti::InterruptHandler<embassy_stm32::interrupt::typelevel::EXTI0>;
    EXTI1 => embassy_stm32::exti::InterruptHandler<embassy_stm32::interrupt::typelevel::EXTI1>;
    EXTI2 => embassy_stm32::exti::InterruptHandler<embassy_stm32::interrupt::typelevel::EXTI2>;
});

type M = CriticalSectionRawMutex;

static NEW_EVENT_SIGNAL: Signal<M, ()> = Signal::new();
static EVENT_SIGNALS: [Signal<M, Event>; 3] = [Signal::new(), Signal::new(), Signal::new()];

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
    spawner.spawn(rotary_switch_task(p.PA0, p.EXTI0)).unwrap();
    spawner
        .spawn(rotary_encoder_task(p.PA1, p.EXTI1, p.PA2, p.EXTI2))
        .unwrap();

    let mut led = Output::new(p.PC13, Level::High, Speed::Low);

    let uart = Uart::new(p.USART1, p.PA10, p.PA9, Irqs, p.DMA1_CH4, p.DMA1_CH5, {
        let mut config = embassy_stm32::usart::Config::default();
        config.baudrate = 4_500_000;
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
        let new_bytes_read = uart_rx.read(&mut buffer[buffer_bytes..]).await.unwrap();
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
async fn rotary_switch_task(pin: Peri<'static, PA0>, exti: Peri<'static, EXTI0>) {
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
    dt: Peri<'static, PA1>,
    dt_exti: Peri<'static, EXTI1>,
    clk: Peri<'static, PA2>,
    clk_exti: Peri<'static, EXTI2>,
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
