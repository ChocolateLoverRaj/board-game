use embassy_sync::{blocking_mutex::raw::RawMutex, signal::Signal};
use embedded_graphics::{
    mono_font::{MonoFont, MonoTextStyleBuilder, iso_8859_16::FONT_7X14},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::{Baseline, Text},
};
use esp_hal::{
    gpio::interconnect::PeripheralOutput,
    i2c::{self, master::I2c},
    time::Rate,
};
use ssd1306::{
    I2CDisplayInterface, Ssd1306Async, mode::DisplayConfigAsync, prelude::*,
    size::DisplaySize128x64,
};

pub const FONT: &MonoFont = &FONT_7X14;
pub const OPTIONS: &[&str] = &[
    "Back",
    "00:00:00:00:00:00",
    "11:11:11:11:11:11",
    "22:22:22:22:22:22",
    "33:33:33:33:33:33",
    "44:44:44:44:44:44",
    "55:55:55:55:55:55",
    "66:66:66:66:66:66",
    "77:77:77:77:77:77",
    "88:88:88:88:88:88",
    "99:99:99:99:99:99",
    "AA:AA:AA:AA:AA:AA",
    "BB:BB:BB:BB:BB:BB",
    "CC:CC:CC:CC:CC:CC",
    "DD:DD:DD:DD:DD:DD",
    "EE:EE:EE:EE:EE:EE",
    "FF:FF:FF:FF:FF:FF",
];
// ssd1306 doesn't expose these numbers so we can just manually write them
pub const DISPLAY_WIDTH: u32 = 128;
pub const DISPLAY_HEIGHT: u32 = 64;

#[derive(Debug, Default)]
pub struct UiState {
    pub selected_index: usize,
    pub scroll_y: u32,
}

pub async fn render_display<'a>(
    i2c: impl i2c::master::Instance + 'a,
    scl: impl PeripheralOutput<'a>,
    sda: impl PeripheralOutput<'a>,
    signal: &Signal<impl RawMutex, UiState>,
) {
    let i2c = I2c::new(
        i2c,
        i2c::master::Config::default().with_frequency(Rate::from_khz(400)),
    )
    .unwrap()
    .with_scl(scl)
    .with_sda(sda)
    .into_async();
    let mut display = Ssd1306Async::new(
        I2CDisplayInterface::new(i2c),
        DisplaySize128x64,
        DisplayRotation::Rotate0,
    )
    .into_buffered_graphics_mode();
    display.init().await.unwrap();

    let mut ui_state = UiState::default();
    loop {
        display.clear(BinaryColor::Off).unwrap();
        for (index, option) in OPTIONS.iter().enumerate() {
            let is_selected = ui_state.selected_index == index;
            Text::with_baseline(
                option,
                Point::new(
                    0,
                    index as i32 * FONT.character_size.height as i32 - ui_state.scroll_y as i32,
                ),
                MonoTextStyleBuilder::new()
                    .font(FONT)
                    .text_color(if is_selected {
                        BinaryColor::Off
                    } else {
                        BinaryColor::On
                    })
                    .background_color(if is_selected {
                        BinaryColor::On
                    } else {
                        BinaryColor::Off
                    })
                    .build(),
                Baseline::Top,
            )
            .draw(&mut display)
            .unwrap();
        }
        // Draw the scrollbar
        let total_height = OPTIONS.len() as f64 * FONT.character_size.height as f64;
        let display_height = DISPLAY_HEIGHT as f64;
        if total_height > display_height {
            let scrollbar_height = ((display_height / total_height * display_height) as u32).max(1);
            let scrollbar_y = (ui_state.scroll_y as f64 / total_height * display_height) as u32;
            let scrollbar_width = 1;
            Rectangle::new(
                Point::new((DISPLAY_WIDTH - scrollbar_width) as i32, scrollbar_y as i32),
                Size::new(scrollbar_width, scrollbar_height),
            )
            .into_styled(
                PrimitiveStyleBuilder::new()
                    .fill_color(BinaryColor::On)
                    .build(),
            )
            .draw(&mut display)
            .unwrap();
        }

        display.flush().await.unwrap();
        ui_state = signal.wait().await;
    }
}
