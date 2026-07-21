use bt_hci::cmd::le::LeSetScanParams;
use bt_hci::controller::ControllerCmdSync;
use bt_hci::param::{AddrKind, BdAddr};
use core::cell::RefCell;
use defmt::{error, info, warn};
use embassy_time::{Duration, Timer};
use heapless::Deque;
use trouble_host::prelude::*;
use trouble_host::{PacketPool, central::Central};

pub async fn scan_for_ftms_device<C, P: PacketPool>(
    central: Central<'_, C, P>,
    handler: &FTMSDeviceScanner,
) -> Result<(AddrKind, BdAddr), ()>
where
    C: Controller + ControllerCmdSync<LeSetScanParams>,
{
    let mut scanner = Scanner::new(central);
    let config = ScanConfig::default();

    let _session = scanner.scan(&config).await;

    // Explicit timer as ScanConfig::timeout does not seem to work properly from testing
    Timer::after(Duration::from_secs(2)).await;

    let res = handler.ftms_devices.borrow().front().cloned().ok_or(());

    match res {
        Ok(found_device) => Ok(found_device),
        Err(err) => {
            let e = defmt::Debug2Format(&err);
            error!("[scan hr devices] error: {:?}", e);
            Err(())
        }
    }
}

pub struct FTMSDeviceScanner {
    seen: RefCell<Deque<BdAddr, 128>>,
    ftms_devices: RefCell<Deque<(AddrKind, BdAddr), 8>>,
}

impl FTMSDeviceScanner {
    pub fn new() -> Self {
        Self {
            seen: RefCell::new(Deque::new()),
            ftms_devices: RefCell::new(Deque::new()),
        }
    }
}

impl EventHandler for FTMSDeviceScanner {
    fn on_adv_reports(&self, mut it: LeAdvReportsIter<'_>) {
        let mut seen = self.seen.borrow_mut();
        let mut ftms_devices = self.ftms_devices.borrow_mut();
        while let Some(Ok(report)) = it.next() {
            if seen.iter().find(|b| b.raw() == report.addr.raw()).is_none() {
                for ad in AdStructure::decode(report.data) {
                    match ad {
                        Ok(ad) => match ad {
                            AdStructure::CompleteServiceUuids16(ids)
                            | AdStructure::IncompleteServiceUuids16(ids)
                                if ids.contains(service::FITNESS_MACHINE.as_le_bytes()) =>
                            {
                                info!("new FTMS-compatible device: {:?}", report.addr);
                                let _ = ftms_devices.push_front((report.addr_kind, report.addr));
                            }

                            _ => { /* Pass devices with other services */ }
                        },
                        Err(err) => {
                            warn!("err when decoding adv data: {:?}", err);
                        }
                    }
                }
                if seen.is_full() {
                    seen.pop_front();
                }
                seen.push_back(report.addr).unwrap();
            }
        }
    }
}
