#![no_std]
#![no_main]

use core::{
    array,
    cell::RefCell,
    iter::{once, repeat, zip},
};

use defmt::{Debug2Format, debug, info, warn};
use display_interface::DisplayError;
use embassy_embedded_hal::{adapter::BlockingAsync, shared_bus::asynch::i2c::I2cDeviceWithConfig};
use embassy_executor::Spawner;
use embassy_futures::join::*;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Delay, Timer};
use embedded_hal::digital::PinState;
use esp_backtrace as _;
use esp_hal::{
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::{self, master::I2c},
    interrupt::software::SoftwareInterruptControl,
    peripherals::{GPIO7, RMT},
    rmt::Rmt,
    spi::{Mode, master::Spi},
    time::Rate,
    timer::timg::TimerGroup,
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

    spawner.spawn(leds_task(p.RMT, p.GPIO7)).unwrap();

    let mut reset_pin = Output::new(p.GPIO8, Level::High, OutputConfig::default());
    reset_pin.set_low();
    Timer::after_nanos(1000).await;
    reset_pin.set_high();
    Timer::after_nanos(37_740).await;

    let i2c = Mutex::<CriticalSectionRawMutex, _>::new(
        I2c::new(p.I2C0, i2c::master::Config::default())
            .unwrap()
            .with_scl(p.GPIO5)
            .with_sda(p.GPIO6)
            .into_async(),
    );

    let mut mcp23017 = Mcp23017::new(
        I2cDeviceWithConfig::new(
            &i2c,
            i2c::master::Config::default().with_frequency(Rate::from_khz(400)),
        ),
        [false, false, false],
        BlockingAsync::new(reset_pin),
        Input::new(p.GPIO1, InputConfig::default().with_pull(Pull::Up)),
        Delay,
    );

    let (mcp23017_runner, ep) = mcp23017.run();
    let rotary_input = RotaryInput2::new();
    let (rotary_input_runner, mut rotary_input) = rotary_input.run(ep.B2, ep.B3);

    join(
        join5(
            mcp23017_runner,
            rotary_input_runner,
            async {
                loop {
                    info!("rotary position: {}", rotary_input.value());
                    rotary_input.watch().await;
                }
            },
            async {
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
            },
            async {
                let mut rotary_button = RotaryButton::new(ep.B1).await;
                loop {
                    rotary_button.wait_until_press().await;
                    info!("rotary button pressed");
                }
            },
        ),
        async {
            let cs_pins = [ep.A0, ep.A1, ep.A2, ep.A3, ep.A4, ep.A5];
            let cs_pins =
                join_array(cs_pins.map(async |pin| pin.into_output(PinState::High).await)).await;
            let spi = LazySharedSpi2::<_, CriticalSectionRawMutex, _>::new(
                Spi::new(p.SPI2, Default::default())
                    .unwrap()
                    .with_sck(p.GPIO4)
                    .with_mosi(p.GPIO3)
                    .with_miso(p.GPIO2)
                    .into_async(),
                cs_pins,
            );
            let mut devices = array::from_fn::<_, 6, _>(|cs_index| {
                AsyncMfrc522::new(
                    SpiRegisterAccess::new(SpiDeviceWithConfig2::new(
                        &spi,
                        cs_index,
                        esp_hal::spi::master::Config::default()
                            .with_frequency(Rate::from_mhz(10))
                            .with_mode(Mode::_0),
                        Delay,
                    )),
                    AsyncPollingWaiterProvider::new(Delay, 25),
                )
            });
            let present_devices = join_array(devices.each_mut().map(async |device| {
                let version = device.version().await.unwrap();
                info!(
                    "version: {:#04X}. chip type: {:#04X}. version: {:#04X}",
                    version,
                    version.get_chip_type(),
                    version.get_version()
                );
                version.get_chip_type() == 0x9 && version.get_version() == 0x2
            }))
            .await;
            info!("present mfrc522 devices: {}", present_devices);
            let n_present_devices = present_devices
                .iter()
                .copied()
                .filter(|is_present| *is_present)
                .count();
            if n_present_devices > 0 {
                // Initialize all present devices
                for device in zip(&mut devices, present_devices)
                    .filter_map(|(device, is_present)| if is_present { Some(device) } else { None })
                {
                    device.soft_reset().await.unwrap();
                    device.init().await.unwrap();
                    device.set_antenna_gain(RxGain::DB18).await.unwrap();
                }
                // Check for cards at one device at a time
                // There are two reasons why we are only checking one device at a time
                // One is that they can interfere with each other
                // Another reason is to not overload the 5V to 3.3V converter on the esp32c3
                loop {
                    let mut ids = Vec::<_, 6>::new();
                    let mut i = 0;
                    while i < n_present_devices {
                        let (device, index) = zip(&mut devices, present_devices)
                            .filter(|(_device, is_present)| *is_present)
                            .nth(i)
                            .unwrap();
                        device.set_antenna_enabled(true).await.unwrap();
                        debug!("Doing  WUPA");
                        match device.card_command(ReqWupA::new(true)).await {
                            Ok(atq_a) => {
                                debug!("Doing SELECT");
                                match device.card_command(Select::new(&atq_a).unwrap()).await {
                                    Ok(uid) => {
                                        // info!("detected uid: {}", uid);
                                        ids.push(uid).unwrap();
                                    }
                                    Err(CardCommandError::CardCommand(e)) => {
                                        warn!("SELECT error: {}", e);
                                    }
                                    result => {
                                        result.unwrap();
                                    }
                                }
                            }
                            Err(CardCommandError::CardCommand(e)) => {
                                debug!("WupA error: {}", e);
                            }
                            result => {
                                result.unwrap();
                            }
                        };
                        device.set_antenna_enabled(false).await.unwrap();
                        i += 1;
                    }
                    info!("scanned ids: {}", ids);
                    Timer::after_millis(500).await;
                }
            }
        },
    )
    .await;
}

// TODO: We probably need a 3.3V to 5V logic level converter
#[embassy_executor::task]
async fn leds_task(rmt: RMT<'static>, pin: GPIO7<'static>) {
    const TOTAL_LEDS: usize = 64;
    let mut buffer = smart_led_buffer!(buffer_size_async(TOTAL_LEDS));
    let mut leds_adapter = SmartLedsAdapterAsync::new(
        Rmt::new(rmt, Rate::from_mhz(80))
            .unwrap()
            .into_async()
            .channel0,
        pin,
        &mut buffer,
    );

    let mut n = 0;
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
        leds_adapter
            .write(brightness(
                repeat(alternate_color)
                    .take(n)
                    .chain(once(RGB::new(255, 0, 0)))
                    .chain(repeat(alternate_color).take(TOTAL_LEDS - n - 1)),
                5,
            ))
            .await
            .unwrap();
        Timer::after_millis(100).await;
    }
}
