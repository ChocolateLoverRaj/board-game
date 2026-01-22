use bt_hci::param::BdAddr;
use core::fmt::{Debug, Write};
use defmt::{Format, info};
use embassy_embedded_hal::{SetConfig, shared_bus::asynch::i2c::I2cDeviceWithConfig};
use embassy_futures::select::{Either, select};
use embassy_sync::{blocking_mutex::raw::RawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Instant, Timer};
use embedded_graphics::{
    mono_font::{MonoFont, MonoTextStyleBuilder, iso_8859_16::FONT_7X14},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};
use embedded_hal_async::i2c::I2c;
use esp_hal::{gpio::Flex, i2c, time::Rate};
use game_pure::{GameState, MainMenuScreen, MainMenuSelectedItem, Screen};
use ssd1306::{
    I2CDisplayInterface, Ssd1306Async, mode::DisplayConfigAsync, prelude::*,
    size::DisplaySize128x64,
};
use strum::{EnumIter, VariantArray};
use trouble_host::Address;

use crate::{
    Element, FlexElement, ListElement, ScrollYElement, TextElement, config::INVERT_SCREEN_INTERVAL,
};

pub const FONT: &MonoFont = &FONT_7X14;
// ssd1306 doesn't expose these numbers so we can just manually write them
pub const DISPLAY_WIDTH: u32 = 128;
pub const DISPLAY_HEIGHT: u32 = 64;

// #[derive(Debug, Default)]
// pub struct UiState {
//     pub selected_index: usize,
//     pub scroll_y: u32,
// }

/// Number of peripheral ids to keep track of when scanning
pub const SCANNING_BUFFER_LEN: usize = 4;

/// Select a device to connect to
#[derive(Debug, Format, Default, Clone)]
pub struct ScanningState {
    pub peripherals: heapless::Vec<Address, SCANNING_BUFFER_LEN>,
    /// `0` is selecting the text that says "Scanning..."
    /// `1..` is for selecting a bluetooth address
    pub selected_index: usize,
    pub scroll_y: u32,
}

impl ScanningState {
    pub fn list<'a, I: I2c>(
        &self,
    ) -> ListElement<impl IntoIterator<Item = impl Element<D<'a, I>>> + Clone> {
        ListElement {
            elements: self.peripherals.iter().enumerate().map(|(i, address)| {
                let is_selected = self.selected_index == i + 1;
                let address = address.clone();
                TextElement {
                    text: {
                        let mut s = heapless::String::<{ 6 * 2 + (6 - 1) }>::new();
                        write!(s, "{address}").unwrap();
                        s
                    },
                    character_style: MonoTextStyleBuilder::new()
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
                }
            }),
        }
    }
}

/// This means that we tried reusing a previously saved bond, but it didn't work.
/// Most likely the peripheral deleted its saved bond.
/// So we can delete our saved bond too and create a new bond, or try again.
#[derive(Debug, Format)]
pub struct ReuseSavedBondErrorState {
    pub address: BdAddr,
    pub option_index: usize,
}

#[derive(EnumIter, VariantArray)]
pub enum ReuseSavedBondErrorOptions {
    DeleteBond,
    Retry,
}

#[derive(Debug, Format)]
pub struct ConnectingUiState {
    pub address: Address,
    pub is_auto: bool,
}

#[derive(Debug, Format, Default)]
pub enum UiState {
    #[default]
    Loading,
    Connecting(ConnectingUiState),
    Scanning(ScanningState),
    Connected(Address),
    ReuseSavedBondError(ReuseSavedBondErrorState),
}

type D<'a, I2c> = Ssd1306Async<
    I2CInterface<I2c>,
    DisplaySize128x64,
    ssd1306::mode::BufferedGraphicsModeAsync<DisplaySize128x64>,
>;

async fn render_ui<I: I2c>(display: &mut D<'_, I>, ui_state: UiState) {
    display.clear(BinaryColor::Off).unwrap();
    match ui_state {
        UiState::Loading => {
            TextElement {
                text: format_args!("Loading"),
                character_style: MonoTextStyleBuilder::new()
                    .font(FONT)
                    .text_color(BinaryColor::On)
                    .build(),
            }
            .draw(display, display.bounding_box())
            .unwrap();
        }
        UiState::Connecting(ConnectingUiState { address, is_auto }) => {
            TextElement {
                text: format_args!("Connecting to {address}\nIs automatic? {is_auto}"),
                character_style: MonoTextStyleBuilder::new()
                    .font(FONT)
                    .text_color(BinaryColor::On)
                    .build(),
            }
            .draw(display, display.bounding_box())
            .unwrap();
        }
        UiState::Connected(address) => {
            TextElement {
                text: format_args!("Connected to {address:?}"),
                character_style: MonoTextStyleBuilder::new()
                    .font(FONT)
                    .text_color(BinaryColor::On)
                    .build(),
            }
            .draw(display, display.bounding_box())
            .unwrap();
        }
        UiState::ReuseSavedBondError(ReuseSavedBondErrorState {
            address,
            option_index,
        }) => {
            Text::with_baseline(
                "Reuse bond failed",
                Point::zero(),
                MonoTextStyleBuilder::new()
                    .font(FONT)
                    .text_color(BinaryColor::On)
                    .build(),
                Baseline::Top,
            )
            .draw(display)
            .unwrap();
            Text::with_baseline(
                "todo:addr",
                Point::new(0, 1 * FONT.character_size.height as i32),
                MonoTextStyleBuilder::new()
                    .font(FONT)
                    .text_color(BinaryColor::On)
                    .build(),
                Baseline::Top,
            )
            .draw(display)
            .unwrap();
            for (index, variant) in ReuseSavedBondErrorOptions::VARIANTS.iter().enumerate() {
                let is_selected = index == option_index;
                Text::with_baseline(
                    match variant {
                        ReuseSavedBondErrorOptions::Retry => "Retry",
                        ReuseSavedBondErrorOptions::DeleteBond => "Delete Bond",
                    },
                    Point::new(0, (2 + index) as i32 * FONT.character_size.height as i32),
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
                .draw(display)
                .unwrap();
            }
        }
        UiState::Scanning(state) => {
            ScrollYElement {
                element: &FlexElement {
                    elements: &[
                        &{
                            let is_selected = state.selected_index == 0;
                            TextElement {
                                text: "Scanning...",
                                character_style: MonoTextStyleBuilder::new()
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
                            }
                        } as &dyn Element<D<'_, _>>,
                        &state.list() as &dyn Element<D<'_, _>>,
                    ],
                    dynamic_element: None,
                },
                scroll_y: state.scroll_y,
                scrollbar_color: BinaryColor::On,
                scrollbar_width: 1,
            }
            .draw(display, display.bounding_box())
            .unwrap();
        }
    }
    display.flush().await.unwrap();
}

pub async fn render_display<'a, Bus>(
    i2c: &Mutex<impl RawMutex, Bus>,
    signal: &Signal<impl RawMutex, UiState>,
) where
    Bus: I2c + SetConfig<Config = i2c::master::Config>,
{
    let i2c = I2cDeviceWithConfig::new(
        i2c,
        i2c::master::Config::default().with_frequency(Rate::from_khz(400)),
    );
    let mut display = Ssd1306Async::new(
        I2CDisplayInterface::new(i2c),
        DisplaySize128x64,
        DisplayRotation::Rotate0,
    )
    .into_buffered_graphics_mode();
    display.init().await.unwrap();

    let mut invert = false;
    let mut last_inverted = Instant::now();
    render_ui(&mut display, Default::default()).await;
    loop {
        match select(
            Timer::at(last_inverted + INVERT_SCREEN_INTERVAL),
            signal.wait(),
        )
        .await
        {
            Either::First(()) => {
                invert = !invert;
                display.set_invert(invert).await.unwrap();
                last_inverted = Instant::now();
            }
            Either::Second(ui_state) => {
                render_ui(&mut display, ui_state).await;
            }
        }
    }
}

async fn render_ui_2<I: I2c>(display: &mut D<'_, I>, game_state: GameState) {
    display.clear(BinaryColor::Off).unwrap();
    match game_state {
        GameState::SettingUp(state) => match state.screen {
            Screen::MainMenu(MainMenuScreen {
                scroll_y,
                selected_item,
            }) => {
                ScrollYElement {
                    element: &ListElement {
                        elements: MainMenuSelectedItem::VARIANTS.into_iter().enumerate().map(
                            |(index, item)| {
                                let is_selected = index == selected_item;
                                TextElement {
                                    text: match item {
                                        MainMenuSelectedItem::StartGame => "Start Game",
                                        MainMenuSelectedItem::Bluetooth => "Bluetooth",
                                    },
                                    character_style: MonoTextStyleBuilder::new()
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
                                }
                            },
                        ),
                    },
                    scroll_y,
                    scrollbar_color: BinaryColor::On,
                    scrollbar_width: 1,
                }
                .draw(display, display.bounding_box())
                .unwrap();
            }
            Screen::Bluetooth(state) => {}
        },
        GameState::Playing(state) => {}
    }
    display.flush().await.unwrap();
}

pub async fn render_display_2<'a, Bus>(
    i2c: &Mutex<impl RawMutex, Bus>,
    signal: &Signal<impl RawMutex, GameState>,
) where
    Bus: I2c + SetConfig<Config = i2c::master::Config>,
{
    let i2c = I2cDeviceWithConfig::new(
        i2c,
        i2c::master::Config::default().with_frequency(Rate::from_khz(400)),
    );
    let mut display = Ssd1306Async::new(
        I2CDisplayInterface::new(i2c),
        DisplaySize128x64,
        DisplayRotation::Rotate0,
    )
    .into_buffered_graphics_mode();
    display.init().await.unwrap();

    let mut invert = false;
    let mut last_inverted = Instant::now();
    loop {
        match select(
            Timer::at(last_inverted + INVERT_SCREEN_INTERVAL),
            signal.wait(),
        )
        .await
        {
            Either::First(()) => {
                invert = !invert;
                display.set_invert(invert).await.unwrap();
                last_inverted = Instant::now();
            }
            Either::Second(game_state) => {
                render_ui_2(&mut display, game_state).await;
            }
        }
    }
}
