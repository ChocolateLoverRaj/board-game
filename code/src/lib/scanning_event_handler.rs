use bt_hci::param::LeAdvReportsIter;
use defmt::warn;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use trouble_host::{
    Address,
    prelude::{AdStructure, EventHandler},
};

use crate::{SERVICE_UUID, liberal_renderer::SCANNING_BUFFER_LEN};

pub const SCAN_CHANNEL_SIZE: usize = SCANNING_BUFFER_LEN;

pub struct ScanningEventHandler<'a> {
    pub channel: &'a Channel<CriticalSectionRawMutex, Address, 1>,
}
impl EventHandler for ScanningEventHandler<'_> {
    fn on_adv_reports(&self, reports: LeAdvReportsIter) {
        reports
            .filter_map(Result::ok)
            .filter(|report| {
                AdStructure::decode(report.data)
                    .filter_map(Result::ok)
                    .any(|ad_structure| {
                        if let AdStructure::ServiceUuids128(uuids) = ad_structure {
                            uuids.contains(SERVICE_UUID.as_raw().try_into().unwrap())
                        } else {
                            false
                        }
                    })
            })
            .for_each(|report| {
                if let Err(e) = self.channel.try_send(Address {
                    addr: report.addr,
                    kind: report.addr_kind,
                }) {
                    warn!("error sending: {}", e);
                };
            });
    }
}
