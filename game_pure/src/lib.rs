#![cfg_attr(not(feature = "std"), no_std)]
pub mod ui;

use core::fmt::Display;

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};
use heapless::index_set::FnvIndexSet;
use strum::VariantArray;
use trouble_host::{
    Address,
    prelude::{AddrKind, BdAddr},
};

use crate::ui::{Screen, SelectedItem};

extern crate alloc;

pub const SCAN_LIST_SIZE: usize = 4;

#[derive(Debug, Clone, Copy)]
pub enum ConnectState {
    Connecting,
    Connected,
}

#[derive(Debug, Clone, Copy)]
pub struct ConnectionStatus {
    pub peripheral_address: BdAddr,
    pub state: ConnectState,
}

#[derive(Debug, Clone)]
pub enum ConnectionAction {
    Scan {
        peripherals: heapless::Vec<BdAddr, SCAN_LIST_SIZE>,
    },
    Connect(ConnectionStatus),
}

#[derive(VariantArray)]
pub enum ConnectingConnectedSelectedItem {
    Back,
    /// Highlight the text that says connecting to ...
    Title,
    Cancel,
}

#[derive(VariantArray)]
pub enum ScanningSelectedItem {
    Back,
    Title,
}

#[derive(Debug, Clone)]
pub enum BluetoothScreen {
    Scanning {
        scroll_y: u32,
        /// See [`ScanningSelectedItem`] for first two items, after that it's one item for each scanned device
        selected_item: usize,
    },
    ConnectingConnected {
        scroll_y: u32,
        /// See [`ConnectingConnectedSelectedItem`]
        selected_item: usize,
    },
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, VariantArray)]
pub enum MainMenuSelectedItem {
    StartGame,
    Bluetooth,
}

#[derive(Debug, Clone)]
pub struct MainMenuScreen {
    pub scroll_y: u32,
    /// See [`MainMenuSelectedItem`]
    pub selected_item: usize,
}

#[derive(Debug, Clone)]
pub enum GameScreen {
    MainMenu(MainMenuScreen),
    Bluetooth(BluetoothScreen),
}

#[derive(Debug, Clone)]
pub struct GameStateSettingUp {
    pub connection_action: ConnectionAction,
    pub screen: GameScreen,
}

#[derive(Debug, Clone, Copy)]
pub enum HitlerState {
    /// It has not been publicly revealed who hitler is.
    Secret,
    /// Hitler was elected as chancellor at a point when 3+ fascist policies were placed, and the fascist team won.
    ElectedChancellor,
    /// Hitler was killed, and the liberal team won.
    Dead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FascistAction {
    /// The president checks another player's party.
    CheckParty,
    /// The president chooses another player to be the next president.
    ChooseNextPresident,
    /// The president chooses another player to kill.
    Kill,
    /// The president examines the top 3 cards.
    /// This action only exists when there are 5-6 players in the game.
    ExamineTop3,
}

fn latest_action(players: u8, fascist_policies_placed: usize) -> Option<FascistAction> {
    match players {
        5 | 6 => match fascist_policies_placed {
            3 => Some(FascistAction::ExamineTop3),
            4 | 5 => Some(FascistAction::Kill),
            _ => None,
        },
        7 | 8 => match fascist_policies_placed {
            2 => Some(FascistAction::CheckParty),
            3 => Some(FascistAction::ChooseNextPresident),
            4 | 5 => Some(FascistAction::Kill),
            _ => None,
        },
        9 | 10 => match fascist_policies_placed {
            1 | 2 => Some(FascistAction::CheckParty),
            3 => Some(FascistAction::ChooseNextPresident),
            4 | 5 => Some(FascistAction::Kill),
            _ => None,
        },
        _ => unreachable!(),
    }
}

impl FascistAction {
    pub fn can_clear_with_button_press(&self) -> bool {
        match self {
            Self::CheckParty | Self::ChooseNextPresident | Self::ExamineTop3 => true,
            Self::Kill => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GameStatePlaying {
    /// The game has 5-10 players. Once the game is started, the number of players currently cannot be adjusted.
    /// However, we could in the future handle changing the number of players mid-game.
    /// We would need to update the `pending_action` field when this happens.
    players: u8,
    connection_status: ConnectionStatus,
    liberal_policies_placed: usize,
    fascist_policies_placed: usize,
    hitler_state: HitlerState,
    election_fail_streak: usize,
    /// The game can give a tip of what to do next on the screen.
    ///
    /// Most of the time, it will say "place a policy or increment the election fail counter".
    ///
    /// If there is a "check another player's party" or "the president chooses the next president"
    /// action, that will be shown on the screen.
    /// The game won't know when the player checked it, so it will keep showing this tip until
    /// a new policy is placed, the fail counter is increased, a button is pressed to dismiss
    /// the hint, or the fascist policy card is removed.
    ///
    /// If there is a "the president kills another player" action, the game will dismiss this
    /// action when a dead character card is detected or the fascist policy card is removed.
    /// This hint cannot be manually dismissed.
    pending_action: bool,
}

impl GameStatePlaying {
    pub fn winner(&self) -> Option<Team> {
        match self.hitler_state {
            HitlerState::Secret => {
                if self.liberal_policies_placed == LIBERAL_BOARD_SLOTS {
                    Some(Team::Liberal)
                } else if self.fascist_policies_placed == FASCIST_BOARD_SLOTS {
                    Some(Team::Fascist)
                } else {
                    None
                }
            }
            HitlerState::ElectedChancellor => Some(Team::Fascist),
            HitlerState::Dead => Some(Team::Liberal),
        }
    }

    // pub fn
}

#[derive(Debug, Clone)]
pub enum GameState {
    SettingUp(GameStateSettingUp),
    Playing(GameStatePlaying),
}

impl GameState {
    /// You can load a auto-connect address if you want
    pub fn new(peripheral_address: Option<BdAddr>) -> Self {
        Self::SettingUp(GameStateSettingUp {
            connection_action: match peripheral_address {
                Some(address) => ConnectionAction::Connect(ConnectionStatus {
                    peripheral_address: address,
                    state: ConnectState::Connecting,
                }),
                None => ConnectionAction::Scan {
                    peripherals: Default::default(),
                },
            },
            screen: GameScreen::MainMenu(MainMenuScreen {
                scroll_y: 0,
                selected_item: 0,
            }),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum BleAction {
    Scan,
    MaintainConnection(BdAddr),
}

pub enum Input {
    Up,
    Down,
    Click,
}

// https://www.secrethitler.com/assets/Secret_Hitler_Rules.pdf
pub const LIBERAL_POLICY_CARDS: usize = 6;
pub const FASCIST_POLICY_CARDS: usize = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuraLedColor {
    /// A blueish color for the liberal board and a reddish color for the fascist board, or something else if the theme is different
    BoardSpecific,
    /// When the liberals win, the fascist board aura also turns blue
    LiberalWin,
    /// When the fascists win, the liberal board aura also turns red
    FascistWin,
}

#[derive(Debug, Clone)]
pub struct LedsDisplay {
    pub aura_led_color: AuraLedColor,
    /// The number of liberal policy LEDs that are lit up
    pub liberal_policy_leds: usize,
    /// The number of fascist policy LEDs that are lit up
    pub fascist_policy_leds: usize,
    /// The number of election tracker LEDs that are lit up
    pub election_tracker_leds: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Team {
    Liberal,
    Fascist,
}

/// Uniquely identifies one of the 17 policy cards
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PolicyCardId {
    pub team: Team,
    pub id: usize,
}

pub const LIBERAL_BOARD_SLOTS: usize = 5;
pub const FASCIST_BOARD_SLOTS: usize = 6;

/// Due to how close the NFC readers are to each other, we do not have 100% confident detection of which slot a policy was placed in.
/// But we can be sure about exactly which policy cards are placed on each board.  
///
/// Note that players can physically place policy cards on the wrong board, such as placing a liberal policy on the fascist board.
#[derive(Debug, Clone)]
pub struct DetectedPolicyCards {
    // FnvIndexSet requires a power of two for the capacity
    pub liberal: FnvIndexSet<PolicyCardId, { LIBERAL_BOARD_SLOTS.next_power_of_two() }>,
    pub fascist: FnvIndexSet<PolicyCardId, { FASCIST_BOARD_SLOTS.next_power_of_two() }>,
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug)]
pub enum SecretRole {
    /// There are up to 6 liberals
    Liberal,
    /// There are up to 3 fascists + 1 hitler
    Fascist,
    /// There is always exactly 1 hitler
    Hitler,
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug)]
pub struct CharacterCardId {
    pub secret_role: SecretRole,
    pub id: usize,
}

impl GameState {
    pub fn ble_action(&self) -> BleAction {
        match self {
            Self::SettingUp(state) => match &state.connection_action {
                ConnectionAction::Scan { peripherals: _ } => BleAction::Scan,
                ConnectionAction::Connect(status) => {
                    BleAction::MaintainConnection(status.peripheral_address)
                }
            },
            Self::Playing(state) => {
                BleAction::MaintainConnection(state.connection_status.peripheral_address)
            }
        }
    }

    fn ble_connection_status_mut(&mut self) -> Option<&mut ConnectionStatus> {
        match self {
            Self::SettingUp(state) => match &mut state.connection_action {
                ConnectionAction::Connect(status) => Some(status),
                ConnectionAction::Scan { peripherals: _ } => None,
            },
            Self::Playing(state) => Some(&mut state.connection_status),
        }
    }

    pub fn ble_connected(&mut self) {
        self.ble_connection_status_mut()
            .expect("game should be trying to maintain a connection and not be scanning")
            .state = ConnectState::Connected;
    }

    pub fn ble_disconnected(&mut self) {
        self.ble_connection_status_mut()
            .expect("game should be trying to maintain a connection and not be scanning")
            .state = ConnectState::Connecting;
    }

    pub fn ble_peripheral_found(&mut self, address: BdAddr) {
        match self {
            Self::SettingUp(state) => match &mut state.connection_action {
                ConnectionAction::Scan { peripherals } => {
                    if !peripherals.contains(&address) {
                        if let Err(address) = peripherals.push(address) {
                            #[cfg(feature = "defmt")]
                            defmt::warn!(
                                "Failed to push address {} to list of scanned peripherals because the list is full. Consider rebuilding with a larger max size.",
                                address
                            );
                        }
                    }
                }
                ConnectionAction::Connect(_) => {
                    unreachable!("this function must be called while scanning");
                }
            },
            Self::Playing(_) => unreachable!("this function must be called while scanning"),
        }
    }

    pub fn process_input(&mut self, input: Input) {
        match self {
            Self::SettingUp(state) => match &mut state.screen {
                GameScreen::MainMenu(screen) => match input {
                    Input::Click => match MainMenuSelectedItem::VARIANTS[screen.selected_item] {
                        MainMenuSelectedItem::StartGame => match &state.connection_action {
                            ConnectionAction::Connect(connection_status) => {
                                *self = GameState::Playing(GameStatePlaying {
                                    players: 10, // TODO: Configure this in settings
                                    connection_status: *connection_status,
                                    liberal_policies_placed: 0,
                                    fascist_policies_placed: 0,
                                    hitler_state: HitlerState::Secret,
                                    election_fail_streak: 0,
                                    pending_action: false,
                                });
                            }
                            ConnectionAction::Scan { peripherals: _ } => {
                                state.screen = GameScreen::Bluetooth(BluetoothScreen::Scanning {
                                    scroll_y: 0, // TODO: make sure it's visible
                                    selected_item: ScanningSelectedItem::Title as usize,
                                });
                            }
                        },
                        MainMenuSelectedItem::Bluetooth => {
                            state.screen = GameScreen::Bluetooth(BluetoothScreen::Scanning {
                                scroll_y: 0, // TODO: make sure it's visible
                                selected_item: ScanningSelectedItem::Title as usize,
                            });
                        }
                    },
                    Input::Down => {
                        screen.selected_item = screen
                            .selected_item
                            .saturating_add(1)
                            .min(MainMenuSelectedItem::VARIANTS.len() - 1);
                        // TODO: Adjust scroll
                    }
                    Input::Up => {
                        screen.selected_item = screen.selected_item.saturating_sub(1);
                        // TODO: Adjust scroll
                    }
                },
                GameScreen::Bluetooth(BluetoothScreen::Scanning {
                    scroll_y,
                    selected_item,
                }) => {
                    let peripherals = match &state.connection_action {
                        ConnectionAction::Scan { peripherals } => peripherals,
                        ConnectionAction::Connect(_) => unreachable!(),
                    };
                    match input {
                        Input::Click => {
                            if *selected_item < ScanningSelectedItem::VARIANTS.len() {
                                match ScanningSelectedItem::VARIANTS[*selected_item] {
                                    ScanningSelectedItem::Back => {
                                        state.screen = GameScreen::MainMenu(MainMenuScreen {
                                            scroll_y: {
                                                // TODO: Make sure it's visible
                                                0
                                            },
                                            selected_item: MainMenuSelectedItem::Bluetooth as usize,
                                        });
                                    }
                                    ScanningSelectedItem::Title => {}
                                }
                            } else {
                                state.connection_action =
                                    ConnectionAction::Connect(ConnectionStatus {
                                        peripheral_address: peripherals
                                            [*selected_item - ScanningSelectedItem::VARIANTS.len()],
                                        state: ConnectState::Connecting,
                                    });
                                state.screen =
                                    GameScreen::Bluetooth(BluetoothScreen::ConnectingConnected {
                                        scroll_y: 0, // TODO: make sure it's visible
                                        selected_item: ConnectingConnectedSelectedItem::Title
                                            as usize,
                                    });
                            }
                        }
                        Input::Down => {
                            *selected_item = selected_item
                                .saturating_add(1)
                                .min(ScanningSelectedItem::VARIANTS.len() + peripherals.len() - 1);
                            // TODO: Make sure it's visible
                        }
                        Input::Up => {
                            *selected_item = selected_item.saturating_sub(1);
                            // TODO: Make sure it's visible
                        }
                    }
                }
                GameScreen::Bluetooth(BluetoothScreen::ConnectingConnected {
                    scroll_y,
                    selected_item,
                }) => {
                    match input {
                        Input::Click => {
                            match ConnectingConnectedSelectedItem::VARIANTS[*selected_item] {
                                ConnectingConnectedSelectedItem::Back => {
                                    state.screen = GameScreen::MainMenu(MainMenuScreen {
                                        scroll_y: {
                                            // TODO: Make sure it's visible
                                            0
                                        },
                                        selected_item: MainMenuSelectedItem::Bluetooth as usize,
                                    });
                                }
                                ConnectingConnectedSelectedItem::Title => {}
                                ConnectingConnectedSelectedItem::Cancel => {
                                    state.connection_action = ConnectionAction::Scan {
                                        peripherals: Default::default(),
                                    };
                                    state.screen =
                                        GameScreen::Bluetooth(BluetoothScreen::Scanning {
                                            scroll_y: 0,
                                            selected_item: 0,
                                        });
                                }
                            }
                        }
                        Input::Down => {
                            *selected_item = selected_item
                                .saturating_add(1)
                                .min(ConnectingConnectedSelectedItem::VARIANTS.len() - 1);
                            // TODO: adjust scroll
                        }
                        Input::Up => {
                            *selected_item = selected_item.saturating_sub(1);
                            // TODO: adjust scroll
                        }
                    }
                }
            },
            Self::Playing(state) => {
                if state.pending_action
                    && latest_action(state.players, state.fascist_policies_placed)
                        .unwrap()
                        .can_clear_with_button_press()
                {
                    state.pending_action = false;
                }
            }
        }
    }

    pub fn get_leds(&self) -> LedsDisplay {
        match self {
            Self::SettingUp(_) => LedsDisplay {
                aura_led_color: AuraLedColor::BoardSpecific,
                liberal_policy_leds: 0,
                fascist_policy_leds: 0,
                election_tracker_leds: 0,
            },
            Self::Playing(state) => LedsDisplay {
                aura_led_color: match state.winner() {
                    Some(Team::Liberal) => AuraLedColor::LiberalWin,
                    Some(Team::Fascist) => AuraLedColor::FascistWin,
                    None => AuraLedColor::BoardSpecific,
                },
                liberal_policy_leds: state.liberal_policies_placed,
                fascist_policy_leds: state.fascist_policies_placed,
                election_tracker_leds: state.election_fail_streak,
            },
        }
    }

    pub fn screen(&self) -> Option<Screen<String, Vec<String>>> {
        match self {
            Self::SettingUp(state) => match state.screen {
                GameScreen::MainMenu(MainMenuScreen {
                    scroll_y,
                    selected_item,
                }) => {
                    Some(Screen {
                        title: "Setup".into(),
                        can_go_back: false,
                        items: MainMenuSelectedItem::VARIANTS
                            .iter()
                            .map(|item| {
                                match item {
                                    MainMenuSelectedItem::StartGame => "Start Game",
                                    MainMenuSelectedItem::Bluetooth => "Bluetooth",
                                }
                                .into()
                            })
                            .collect(),
                        selected_item: SelectedItem::Item(0),
                    })
                    // None
                }
                GameScreen::Bluetooth(BluetoothScreen::Scanning {
                    scroll_y,
                    selected_item,
                }) => Some(Screen {
                    title: "Bluetooth".into(),
                    can_go_back: true,
                    items: match &state.connection_action {
                        ConnectionAction::Scan { peripherals } => peripherals,
                        _ => unreachable!(),
                    }
                    .iter()
                    .copied()
                    .map(|addr| {
                        Address {
                            addr,
                            kind: AddrKind::RANDOM,
                        }
                        .to_string()
                    })
                    .collect(),
                    selected_item: SelectedItem::Item(0),
                }),
                _ => None,
            },
            _ => None,
        }
    }

    /// If this returns `true`, you should use continuously poll NFC readers to detect policy cards and dead character cards.
    pub fn should_scan_cards(&self) -> bool {
        match self {
            Self::SettingUp(_) => false,
            Self::Playing(_) => true,
        }
    }

    /// Completely replaces the previous list of detected policy cards with the new list.
    /// Caller should handle debouncing if necessary.
    pub fn update_scanned_policy_cards(&mut self, cards: DetectedPolicyCards) {
        let state = match self {
            Self::Playing(state) => state,
            Self::SettingUp(_) => {
                unreachable!("should not care about scanned policy cards during setup")
            }
        };
        // For now we will not care which board a policy is placed on.
        // We will process it anyways
        let mut liberal_policies_placed = 0;
        let mut fascist_policies_placed = 0;
        for card in [cards.liberal.iter(), cards.fascist.iter()]
            .into_iter()
            .flatten()
        {
            *match card.team {
                Team::Liberal => &mut liberal_policies_placed,
                Team::Fascist => &mut fascist_policies_placed,
            } += 1;
        }

        // Reset election tracker if any new policy was placed
        let new_policy_card_placed = liberal_policies_placed > state.liberal_policies_placed
            || fascist_policies_placed > state.fascist_policies_placed;
        if new_policy_card_placed {
            state.election_fail_streak = 0;
        }

        // Clear the action hint if any new policy was placed
        if liberal_policies_placed > state.liberal_policies_placed {
            state.pending_action = false;
        }
        if fascist_policies_placed > state.fascist_policies_placed {
            state.pending_action = latest_action(state.players, fascist_policies_placed).is_some();
        }

        // TODO: Undo some stuff if a policy was removed. The only reason policies are removed is if they were placed on accident.

        state.liberal_policies_placed = liberal_policies_placed;
        state.fascist_policies_placed = fascist_policies_placed;
    }

    /// Whenever a character dies, the player scans their character card in the dead character area, and then removes their character card from the scan area.
    /// So there is no undoing this scan. This is why this function is called *process* and not *update*.
    /// Up to two characters can die in one game.
    pub fn process_dead_character(&mut self, character: CharacterCardId) {
        let state = match self {
            Self::Playing(state) => state,
            Self::SettingUp(_) => {
                unreachable!("should not care about scanned dead character cards during setup")
            }
        };
        if latest_action(state.players, state.fascist_policies_placed) == Some(FascistAction::Kill)
            && state.pending_action
        {
            match character.secret_role {
                SecretRole::Hitler => {
                    state.hitler_state = HitlerState::Dead;
                }
                _ => {}
            }
            state.pending_action = false;
        } else {
            #[cfg(feature = "defmt")]
            defmt::warn!(
                "Processed dead character {} when no one should have been killed.",
                character
            );
        }
    }

    pub fn display_action_hint(&self) -> Option<FascistAction> {
        match self {
            Self::Playing(state) => {
                if state.pending_action {
                    latest_action(state.players, state.fascist_policies_placed)
                } else {
                    None
                }
            }
            Self::SettingUp(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use trouble_host::prelude::BdAddr;

    use super::*;

    #[test]
    fn six_fascist_policies() {
        let mut state = GameState::new(None);
        // Enter bluetooth menu
        state.process_input(Input::Down);
        state.process_input(Input::Click);

        // Simulate a bluetooth device showing up
        assert_eq!(state.ble_action(), BleAction::Scan);
        let address = BdAddr::new([0x00, 0x01, 0x02, 0x03, 0x04, 0x05]);
        state.ble_peripheral_found(address);

        // Select that bluetooth device
        state.process_input(Input::Down);
        state.process_input(Input::Click);

        // Go back to main menu
        state.process_input(Input::Up);
        state.process_input(Input::Click);

        // Start the game
        state.process_input(Input::Up);
        state.process_input(Input::Click);

        assert!(matches!(state, GameState::Playing(_)));
        assert_eq!(state.ble_action(), BleAction::MaintainConnection(address));
        assert_eq!(state.should_scan_cards(), true);

        // A fascist policy is placed
        state.update_scanned_policy_cards(DetectedPolicyCards {
            liberal: [].into_iter().collect(),
            fascist: [PolicyCardId {
                team: Team::Fascist,
                id: 0,
            }]
            .into_iter()
            .collect(),
        });
        // The hint should show up
        assert_eq!(state.display_action_hint(), Some(FascistAction::CheckParty));
        // Manually dismiss the hint
        state.process_input(Input::Click);
        assert_eq!(state.display_action_hint(), None);

        // A liberal policy is placed
        state.update_scanned_policy_cards(DetectedPolicyCards {
            liberal: [PolicyCardId {
                team: Team::Liberal,
                id: 0,
            }]
            .into_iter()
            .collect(),
            fascist: [PolicyCardId {
                team: Team::Fascist,
                id: 0,
            }]
            .into_iter()
            .collect(),
        });
        assert_eq!(state.display_action_hint(), None);

        // Fascist policy placed
        state.update_scanned_policy_cards(DetectedPolicyCards {
            liberal: [PolicyCardId {
                team: Team::Liberal,
                id: 0,
            }]
            .into_iter()
            .collect(),
            fascist: [
                PolicyCardId {
                    team: Team::Fascist,
                    id: 0,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 1,
                },
            ]
            .into_iter()
            .collect(),
        });
        // The hint should show up
        assert_eq!(state.display_action_hint(), Some(FascistAction::CheckParty));
        // Manually dismiss the hint
        state.process_input(Input::Click);
        assert_eq!(state.display_action_hint(), None);

        // Liberal policy placed
        state.update_scanned_policy_cards(DetectedPolicyCards {
            liberal: [
                PolicyCardId {
                    team: Team::Liberal,
                    id: 0,
                },
                PolicyCardId {
                    team: Team::Liberal,
                    id: 1,
                },
            ]
            .into_iter()
            .collect(),
            fascist: [
                PolicyCardId {
                    team: Team::Fascist,
                    id: 0,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 1,
                },
            ]
            .into_iter()
            .collect(),
        });
        assert_eq!(state.display_action_hint(), None);

        // Fascist policy placed
        state.update_scanned_policy_cards(DetectedPolicyCards {
            liberal: [
                PolicyCardId {
                    team: Team::Liberal,
                    id: 0,
                },
                PolicyCardId {
                    team: Team::Liberal,
                    id: 1,
                },
            ]
            .into_iter()
            .collect(),
            fascist: [
                PolicyCardId {
                    team: Team::Fascist,
                    id: 0,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 1,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 2,
                },
            ]
            .into_iter()
            .collect(),
        });
        // The hint should show up
        assert_eq!(
            state.display_action_hint(),
            Some(FascistAction::ChooseNextPresident)
        );
        // Manually dismiss the hint
        state.process_input(Input::Click);
        assert_eq!(state.display_action_hint(), None);

        // Fascist policy placed
        state.update_scanned_policy_cards(DetectedPolicyCards {
            liberal: [
                PolicyCardId {
                    team: Team::Liberal,
                    id: 0,
                },
                PolicyCardId {
                    team: Team::Liberal,
                    id: 1,
                },
            ]
            .into_iter()
            .collect(),
            fascist: [
                PolicyCardId {
                    team: Team::Fascist,
                    id: 0,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 1,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 2,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 3,
                },
            ]
            .into_iter()
            .collect(),
        });
        // The hint should show up
        assert_eq!(state.display_action_hint(), Some(FascistAction::Kill));
        // A liberal is killed
        state.process_dead_character(CharacterCardId {
            secret_role: SecretRole::Liberal,
            id: 0,
        });
        assert_eq!(state.display_action_hint(), None);

        // Fascist policy placed
        state.update_scanned_policy_cards(DetectedPolicyCards {
            liberal: [
                PolicyCardId {
                    team: Team::Liberal,
                    id: 0,
                },
                PolicyCardId {
                    team: Team::Liberal,
                    id: 1,
                },
            ]
            .into_iter()
            .collect(),
            fascist: [
                PolicyCardId {
                    team: Team::Fascist,
                    id: 0,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 1,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 2,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 3,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 4,
                },
            ]
            .into_iter()
            .collect(),
        });
        // The hint should show up
        assert_eq!(state.display_action_hint(), Some(FascistAction::Kill));
        // A fascist is killed
        state.process_dead_character(CharacterCardId {
            secret_role: SecretRole::Fascist,
            id: 0,
        });
        assert_eq!(state.display_action_hint(), None);

        // Fascist policy placed
        state.update_scanned_policy_cards(DetectedPolicyCards {
            liberal: [
                PolicyCardId {
                    team: Team::Liberal,
                    id: 0,
                },
                PolicyCardId {
                    team: Team::Liberal,
                    id: 1,
                },
            ]
            .into_iter()
            .collect(),
            fascist: [
                PolicyCardId {
                    team: Team::Fascist,
                    id: 0,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 1,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 2,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 3,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 4,
                },
                PolicyCardId {
                    team: Team::Fascist,
                    id: 5,
                },
            ]
            .into_iter()
            .collect(),
        });
        // Fascists win
        assert_eq!(state.get_leds().aura_led_color, AuraLedColor::FascistWin);
    }
}
