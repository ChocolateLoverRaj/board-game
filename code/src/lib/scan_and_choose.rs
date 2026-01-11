use bt_hci::{
    cmd::{
        controller_baseband::{
            HostBufferSize, HostNumberOfCompletedPackets, Reset, SetControllerToHostFlowControl,
            SetEventMask, SetEventMaskPage2,
        },
        info::ReadBdAddr,
        le::{
            LeAddDeviceToFilterAcceptList, LeClearFilterAcceptList, LeConnUpdate,
            LeCreateConnCancel, LeEnableEncryption, LeLongTermKeyRequestReply, LeReadBufferSize,
            LeReadFilterAcceptListSize, LeSetAdvEnable, LeSetEventMask, LeSetExtAdvEnable,
            LeSetExtScanEnable, LeSetRandomAddr, LeSetScanEnable, LeSetScanParams,
        },
        link_control::Disconnect,
    },
    controller::{ControllerCmdAsync, ControllerCmdSync},
};
use defmt::info;
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_sync::{blocking_mutex::raw::RawMutex, channel::Channel, signal::Signal};
use embassy_time::Duration;
use trouble_host::{Controller, PacketPool, prelude::*, scan::Scanner};

use crate::{
    Direction, RotaryButton, RotaryInput, ScanningEventHandler,
    liberal_renderer::{ScanningState, UiState},
};

pub async fn scan_and_choose<
    C: Controller
        + ControllerCmdSync<Disconnect>
        + ControllerCmdSync<SetEventMask>
        + ControllerCmdSync<SetEventMaskPage2>
        + ControllerCmdSync<LeSetEventMask>
        + ControllerCmdSync<LeSetRandomAddr>
        + ControllerCmdSync<LeReadFilterAcceptListSize>
        + ControllerCmdSync<HostBufferSize>
        + ControllerCmdAsync<LeConnUpdate>
        + ControllerCmdSync<SetControllerToHostFlowControl>
        + for<'t> ControllerCmdSync<LeSetAdvEnable>
        + for<'t> ControllerCmdSync<LeSetExtAdvEnable<'t>>
        + for<'t> ControllerCmdSync<HostNumberOfCompletedPackets<'t>>
        + ControllerCmdSync<LeSetScanEnable>
        + ControllerCmdSync<LeSetExtScanEnable>
        + ControllerCmdSync<Reset>
        + ControllerCmdSync<LeCreateConnCancel>
        + ControllerCmdSync<LeReadBufferSize>
        + ControllerCmdSync<LeLongTermKeyRequestReply>
        + ControllerCmdAsync<LeEnableEncryption>
        + ControllerCmdSync<ReadBdAddr>
        + ControllerCmdSync<LeSetScanParams>
        + ControllerCmdSync<LeSetScanEnable>
        + ControllerCmdSync<LeClearFilterAcceptList>
        + ControllerCmdSync<LeAddDeviceToFilterAcceptList>,
    P: PacketPool,
>(
    runner: &mut Runner<'_, C, P>,
    scanner: &mut Scanner<'_, C, P>,
    rotary_input: &mut RotaryInput<'_>,
    rotary_button: &mut RotaryButton<'_>,
    signal: &Signal<impl RawMutex, UiState>,
) -> Address {
    let channel = Channel::new();
    match select(
        runner.run_with_handler(&ScanningEventHandler { channel: &channel }),
        async {
            let mut scanning_state = ScanningState::default();
            signal.signal(UiState::Scanning(scanning_state.clone()));
            let _session = scanner
                .scan(&ScanConfig {
                    active: true,
                    phys: PhySet::M1,
                    interval: Duration::from_secs(1),
                    window: Duration::from_secs(1),
                    ..Default::default()
                })
                .await
                .unwrap();
            let mut partial_step_position = 0;
            // 2 is naturally how the rotary encoder physically "snaps"
            let steps_per_increment = 2;
            loop {
                use Either3::*;
                match select3(
                    rotary_input.next(),
                    channel.receive(),
                    rotary_button.wait_until_press(),
                )
                .await
                {
                    First(direction) => {
                        info!("rotary direction: {}", direction);
                        partial_step_position += match direction {
                            Direction::Clockwise => 1,
                            Direction::CounterClockwise => -1,
                        };
                        let selected_index_changed = if partial_step_position >= steps_per_increment
                        {
                            scanning_state.selected_index = scanning_state
                                .selected_index
                                .saturating_add(1)
                                .min(1 + scanning_state.peripherals.len() - 1);
                            true
                        } else if partial_step_position <= -steps_per_increment {
                            scanning_state.selected_index =
                                scanning_state.selected_index.saturating_sub(1);
                            true
                        } else {
                            false
                        };
                        if selected_index_changed {
                            // TODO: Scroll into view
                            partial_step_position = 0;
                            signal.signal(UiState::Scanning(scanning_state.clone()));
                        }
                    }
                    Second(address) => {
                        // TODO: Maybe remove some peripherals if we haven't seen them for a while
                        if !scanning_state.peripherals.contains(&address) {
                            if scanning_state.peripherals.is_full() {
                                scanning_state.peripherals.remove(0);
                            }
                            scanning_state.peripherals.push(address).unwrap();
                            signal.signal(UiState::Scanning(scanning_state.clone()));
                        }
                    }
                    Third(_) => {
                        if scanning_state.selected_index > 0 {
                            break;
                        }
                    }
                }
            }
            scanning_state.peripherals[scanning_state.selected_index - 1]
        },
    )
    .await
    {
        Either::First(_) => unreachable!(),
        Either::Second(selected_index) => selected_index,
    }
}
