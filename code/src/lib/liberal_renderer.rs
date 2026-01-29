use bt_hci::param::{AddrKind, BdAddr};
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
use game_pure::{
    BluetoothScreen, ConnectState, ConnectionAction, GameScreen, GameState, MainMenuScreen,
    MainMenuSelectedItem, ScanningSelectedItem,
};
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

type D<'a, I2c> = Ssd1306Async<
    I2CInterface<I2c>,
    DisplaySize128x64,
    ssd1306::mode::BufferedGraphicsModeAsync<DisplaySize128x64>,
>;

async fn render_ui_2<I: I2c>(display: &mut D<'_, I>, game_state: GameState) {
    display.clear(BinaryColor::Off).unwrap();
    match game_state {
        GameState::SettingUp(state) => match state.screen {
            GameScreen::MainMenu(MainMenuScreen {
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
            GameScreen::Bluetooth(BluetoothScreen::Scanning {
                scroll_y,
                selected_item,
            }) => {
                ScrollYElement {
                    element: &FlexElement {
                        elements: &[
                            &ListElement {
                                elements: ScanningSelectedItem::VARIANTS.iter().enumerate().map(
                                    |(i, item)| {
                                        let is_selected = selected_item == i;
                                        TextElement {
                                            text: match item {
                                                ScanningSelectedItem::Back => "Back",
                                                ScanningSelectedItem::Title => "Bluetooth",
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
                            } as &dyn Element<D<'_, _>>,
                            &ListElement {
                                elements: match state.connection_action {
                                    ConnectionAction::Scan { peripherals } => peripherals,
                                    _ => unreachable!(),
                                }
                                .iter()
                                .enumerate()
                                .map(|(i, item)| {
                                    let is_selected =
                                        selected_item == ScanningSelectedItem::VARIANTS.len() + i;
                                    TextElement {
                                        text: Address {
                                            addr: *item,
                                            kind: AddrKind::RANDOM,
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
                            } as &dyn Element<D<'_, _>>,
                        ],
                        dynamic_element: None,
                    },
                    scroll_y,
                    scrollbar_color: BinaryColor::On,
                    scrollbar_width: 1,
                }
                .draw(display, display.bounding_box())
                .unwrap();
            }
            GameScreen::Bluetooth(BluetoothScreen::ConnectingConnected {
                scroll_y,
                selected_item,
            }) => {
                TextElement {
                    text: match match state.connection_action {
                        ConnectionAction::Connect(status) => status,
                        _ => unreachable!(),
                    }
                    .state
                    {
                        ConnectState::Connecting => "Connecting",
                        ConnectState::Connected => "Connected",
                    },
                    character_style: MonoTextStyleBuilder::new()
                        .font(FONT)
                        .text_color(BinaryColor::On)
                        .build(),
                }
                .draw(display, display.bounding_box())
                .unwrap();
            }
        },
        GameState::Playing(state) => {
            TextElement {
                text: "Playing Game",
                character_style: MonoTextStyleBuilder::new()
                    .font(FONT)
                    .text_color(BinaryColor::On)
                    .build(),
            }
            .draw(display, display.bounding_box())
            .unwrap();
        }
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
