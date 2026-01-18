use core::future::pending;

use bt_hci::controller::ExternalController;
use embassy_futures::{
    join::{join, join3},
    select::{Either, Select, select, select3},
};
use embassy_sync::channel::Channel;
use embassy_time::Duration;
use esp_hal::{efuse::Efuse, peripherals::BT};
use esp_radio::{Controller, ble::controller::BleConnector};
use trouble_host::{
    Address, Host, HostResources, IoCapabilities, Stack,
    prelude::{Central, DefaultPacketPool, PhySet, Runner, ScanConfig},
    scan::{ScanSession, Scanner},
};

use crate::{BLE_SLOTS, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, ScanChannel, ScanningEventHandler};

type Resources = HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX>;

pub struct BleRef0 {
    resources: Resources,
}

impl BleRef0 {
    pub fn new() -> Self {
        Self {
            resources: Resources::new(),
        }
    }
}

pub struct BleRef1<'a> {
    stack: Stack<'a, ExternalController<BleConnector<'a>, BLE_SLOTS>, DefaultPacketPool>,
}

impl<'a> BleRef1<'a> {
    pub fn new(controller: &'a Controller<'a>, bt: BT<'a>, ref_0: &'a mut BleRef0) -> Self {
        let connector = BleConnector::new(controller, bt, Default::default()).unwrap();
        let controller = ExternalController::<_, BLE_SLOTS>::new(connector);
        let our_address = Address::random(Efuse::mac_address());
        let stack = trouble_host::new(controller, &mut ref_0.resources)
            .set_random_address(our_address)
            .set_io_capabilities(IoCapabilities::DisplayYesNo);
        Self { stack }
    }
}

type MyHost<'a> = Host<'a, ExternalController<BleConnector<'a>, BLE_SLOTS>, DefaultPacketPool>;

pub struct Ble<'a> {
    runner: Runner<'a, ExternalController<BleConnector<'a>, BLE_SLOTS>, DefaultPacketPool>,
    /// Always `Some` unless the value inside is temporarily taken out to be used by something while this is borrowed.
    central:
        Option<Central<'a, ExternalController<BleConnector<'a>, BLE_SLOTS>, DefaultPacketPool>>,
}

impl<'a> Ble<'a> {
    // pub fn new(controller: &'a Controller<'a>, bt: BT<'a>, ref_0: &'a mut BleRef0) -> Self {
    //     let connector = BleConnector::new(controller, bt, Default::default()).unwrap();
    //     let controller = ExternalController::<_, BLE_SLOTS>::new(connector);
    //     let our_address = Address::random(Efuse::mac_address());
    //     let stack = trouble_host::new(controller, &mut ref_0.resources)
    //         .set_random_address(our_address)
    //         .set_io_capabilities(IoCapabilities::DisplayYesNo);
    //     Self { stack }
    // }
    //

    pub fn new(
        host: Host<'a, ExternalController<BleConnector<'a>, BLE_SLOTS>, DefaultPacketPool>,
    ) -> Self {
        Self {
            runner: host.runner,
            central: Some(host.central),
        }
    }

    pub fn scan(&'a mut self) -> BleScanner<'a> {
        BleScanner::new(self)
    }
}

pub struct BleScanner<'a> {
    channel: ScanChannel,
    ble: &'a mut Ble<'a>,
    scanner: Scanner<'a, ExternalController<BleConnector<'a>, BLE_SLOTS>, DefaultPacketPool>,
}

impl<'a> BleScanner<'a> {
    fn new(ble: &'a mut Ble<'a>) -> Self {
        Self {
            channel: ScanChannel::new(),
            scanner: Scanner::new(ble.central.take().unwrap()),
            ble: ble,
        }
    }

    pub fn get(&'a mut self) -> (ScannerRunner<'a>, BleScannerScanner<'a>) {
        (
            ScannerRunner {
                channel: &self.channel,
                runner: &mut self.ble.runner,
                scanner: &mut self.scanner,
            },
            BleScannerScanner {
                channel: &self.channel,
            },
        )
    }
}

pub struct ScannerRunner<'a> {
    channel: &'a ScanChannel,
    runner: &'a mut Runner<'a, ExternalController<BleConnector<'a>, BLE_SLOTS>, DefaultPacketPool>,
    scanner:
        &'a mut Scanner<'a, ExternalController<BleConnector<'a>, BLE_SLOTS>, DefaultPacketPool>,
}

impl ScannerRunner<'_> {
    pub async fn run(&mut self) {
        let a = join(
            self.runner.run_with_handler(&ScanningEventHandler {
                channel: self.channel,
            }),
            async {
                let _scan_session = self
                    .scanner
                    .scan(&ScanConfig {
                        active: true,
                        phys: PhySet::M1,
                        interval: Duration::from_secs(1),
                        window: Duration::from_secs(1),
                        ..Default::default()
                    })
                    .await
                    .unwrap();
                pending::<()>().await;
            },
        )
        .await;
    }
}

pub struct BleScannerScanner<'a> {
    channel: &'a ScanChannel,
}

impl BleScannerScanner<'_> {
    pub async fn next(&mut self) -> Address {
        self.channel.receive().await
    }
}
