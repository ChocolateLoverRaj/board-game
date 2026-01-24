use core::future::pending;

use bt_hci::{
    controller::{Controller, ExternalController},
    param::AddrKind,
};
use defmt::{info, warn};
use embassy_futures::{
    join::join,
    select::{Either, select},
};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, signal::Signal,
};
use embassy_time::Duration;
use esp_hal::{efuse::Efuse, peripherals::BT};
use esp_radio::ble::controller::{BleConnector, BleConnectorError};
use game_pure::ConnectState;
use trouble_host::{
    Address, BleHostError, Host, HostResources, IoCapabilities, PacketPool,
    l2cap::{L2capChannel, L2capChannelConfig},
    prelude::{Central, ConnectConfig, DefaultPacketPool, PhySet, ScanConfig},
    scan::Scanner,
};

use crate::{
    CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, PSM_L2CAP_EXAMPLES, ScanChannel, ScanningEventHandler,
};

#[derive(Debug, Default, PartialEq)]
enum Command {
    #[default]
    Off,
    Scan,
    MaintainConnection(Address),
}

enum CentralOrScanner<'stack, C: Controller, P: PacketPool> {
    Central(Option<Central<'stack, C, P>>),
    Scanner(Option<Scanner<'stack, C, P>>),
}

impl<'stack, C: Controller, P: PacketPool> CentralOrScanner<'stack, C, P> {
    pub fn new(central: Central<'stack, C, P>) -> Self {
        Self::Central(Some(central))
    }

    pub fn central(&mut self) -> &mut Central<'stack, C, P> {
        if let Self::Scanner(scanner) = self {
            *self = Self::Central(Some(scanner.take().unwrap().into_inner()));
        }
        match self {
            Self::Central(central) => central.as_mut().unwrap(),
            _ => unreachable!(),
        }
    }

    pub fn scanner(&mut self) -> &mut Scanner<'stack, C, P> {
        if let Self::Central(central) = self {
            *self = Self::Scanner(Some(Scanner::new(central.take().unwrap())));
        }
        match self {
            Self::Scanner(scanner) => scanner.as_mut().unwrap(),
            _ => unreachable!(),
        }
    }
}

pub struct Ble2 {
    command_signal: Signal<CriticalSectionRawMutex, Command>,
    scan_channel: ScanChannel,
    connection_signal: Signal<CriticalSectionRawMutex, ConnectState>,
}

impl Ble2 {
    pub fn new() -> Self {
        Self {
            command_signal: Signal::new(),
            scan_channel: Channel::new(),
            connection_signal: Signal::new(),
        }
    }

    pub fn run(
        &mut self,
        controller: &esp_radio::Controller,
        bt: BT,
    ) -> (impl Future<Output = ()>, Ble2Api<'_>) {
        let ble = &*self;
        (
            async move {
                let connector = BleConnector::new(&controller, bt, Default::default()).unwrap();
                let controller = ExternalController::<_, 20>::new(connector);
                let our_address = Address::random(Efuse::mac_address());
                let mut resources =
                    HostResources::<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX>::new();
                let stack = trouble_host::new(controller, &mut resources)
                    .set_random_address(our_address)
                    .set_io_capabilities(IoCapabilities::DisplayYesNo);
                let Host {
                    central,
                    mut runner,
                    ..
                } = stack.build();
                let mut central = CentralOrScanner::new(central);

                let mut command = Command::default();
                loop {
                    match select(
                        async {
                            loop {
                                let new_command = ble.command_signal.wait().await;
                                if new_command != command {
                                    break new_command;
                                }
                            }
                        },
                        async {
                            match command {
                                Command::Off => {
                                    info!("stopped running BLE");
                                    pending::<()>().await;
                                }
                                Command::Scan => {
                                    join(
                                        async {
                                            loop {
                                                if let Err(e) = runner
                                                    .run_with_handler(&ScanningEventHandler {
                                                        channel: &ble.scan_channel,
                                                    })
                                                    .await
                                                {
                                                    warn!("BLE error: {}", e);
                                                }
                                            }
                                        },
                                        async {
                                            let _session = loop {
                                                match central
                                                    .scanner()
                                                    .scan(&ScanConfig {
                                                        active: true,
                                                        phys: PhySet::M1,
                                                        interval: Duration::from_secs(1),
                                                        window: Duration::from_secs(1),
                                                        ..Default::default()
                                                    })
                                                    .await
                                                {
                                                    Ok(session) => break session,
                                                    Err(e) => {
                                                        warn!("BLE error: {}", e);
                                                    }
                                                }
                                            };
                                            pending::<()>().await;
                                        },
                                    )
                                    .await;
                                }
                                Command::MaintainConnection(address) => {
                                    join(
                                        async {
                                            loop {
                                                if let Err(e) = runner.run().await {
                                                    warn!("BLE error: {}", e);
                                                }
                                            }
                                        },
                                        async {
                                            let connection = loop {
                                                match central
                                                    .central()
                                                    .connect(&ConnectConfig {
                                                        connect_params: Default::default(),
                                                        scan_config: ScanConfig {
                                                            filter_accept_list: &[(
                                                                AddrKind::RANDOM,
                                                                &address.addr,
                                                            )],
                                                            ..Default::default()
                                                        },
                                                    })
                                                    .await
                                                {
                                                    Ok(connection) => break connection,
                                                    Err(e) => {
                                                        warn!("BLE error: {}", e);
                                                    }
                                                }
                                            };
                                            ble.connection_signal.signal(ConnectState::Connected);
                                            info!("Connected, creating l2cap channel");
                                            const PAYLOAD_LEN: usize = 27;
                                            let config = L2capChannelConfig {
                                                mtu: Some(PAYLOAD_LEN as u16),
                                                ..Default::default()
                                            };
                                            let mut ch1 = L2capChannel::create(
                                                &stack,
                                                &connection,
                                                PSM_L2CAP_EXAMPLES,
                                                &config,
                                            )
                                            .await
                                            .unwrap();
                                            info!("New l2cap channel created, sending some data!");
                                            for i in 0..10 {
                                                let tx = [i; PAYLOAD_LEN];
                                                ch1.send(&stack, &tx).await.unwrap();
                                            }
                                            info!("Sent data, waiting for them to be sent back");
                                            let mut rx = [0; PAYLOAD_LEN];
                                            for i in 0..10 {
                                                let len =
                                                    ch1.receive(&stack, &mut rx).await.unwrap();
                                                assert_eq!(len, rx.len());
                                                assert_eq!(rx, [i; PAYLOAD_LEN]);
                                            }

                                            info!("Received successfully!");
                                            core::future::pending::<()>().await;
                                        },
                                    )
                                    .await;
                                }
                            }
                        },
                    )
                    .await
                    {
                        Either::First(new_command) => {
                            command = new_command;
                        }
                        Either::Second(_) => unreachable!(),
                    };
                }
            },
            Ble2Api { ble: self },
        )
    }
}

pub enum BleEvent {
    PeripheralScanned(Address),
    ConnectionUpdate(ConnectState),
}

pub struct Ble2Api<'a> {
    ble: &'a Ble2,
}

impl Ble2Api<'_> {
    pub fn off(&mut self) {
        self.ble.command_signal.signal(Command::Off);
    }

    pub fn scan(&mut self) {
        self.ble.command_signal.signal(Command::Scan);
    }

    pub async fn next_scanned_address(&mut self) -> Address {
        self.ble.scan_channel.receive().await
    }

    pub fn maintain_connection(&mut self, address: Address) {
        self.ble
            .command_signal
            .signal(Command::MaintainConnection(address));
    }

    pub async fn next(&mut self) -> BleEvent {
        use embassy_futures::select::{Either::*, *};
        match select(
            self.ble.scan_channel.receive(),
            self.ble.connection_signal.wait(),
        )
        .await
        {
            First(address) => BleEvent::PeripheralScanned(address),
            Second(state) => BleEvent::ConnectionUpdate(state),
        }
    }
}
