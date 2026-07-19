#![no_std]
#![no_main]

use core::cell::RefCell;

use bt_hci::cmd::le::LeSetScanParams;
use bt_hci::controller::ControllerCmdSync;
use defmt::{info, unwrap, warn};
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_nrf::mode::Async;
use embassy_nrf::peripherals::RNG;
use embassy_nrf::{bind_interrupts, rng};
use embassy_time::{Duration, Timer};
use heapless::Deque;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use nrf_sdc::{self as sdc, mpsl};
use static_cell::StaticCell;
use trouble_host::prelude::*;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    RNG => rng::InterruptHandler<RNG>;
    EGU0_SWI0 => nrf_sdc::mpsl::LowPrioInterruptHandler;
    CLOCK_POWER => nrf_sdc::mpsl::ClockInterruptHandler;
    RADIO => nrf_sdc::mpsl::HighPrioInterruptHandler;
    TIMER0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
    RTC0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
});

#[embassy_executor::task]
async fn mpsl_task(mpsl: &'static MultiprotocolServiceLayer<'static>) -> ! {
    mpsl.run().await
}

/// How many outgoing L2CAP buffers per link
const L2CAP_TXQ: u8 = 3;

/// How many incoming L2CAP buffers per link
const L2CAP_RXQ: u8 = 3;

fn build_sdc<'d, const N: usize>(
    p: nrf_sdc::Peripherals<'d>,
    rng: &'d mut rng::Rng<Async>,
    mpsl: &'d MultiprotocolServiceLayer,
    mem: &'d mut sdc::Mem<N>,
) -> Result<nrf_sdc::SoftdeviceController<'d>, nrf_sdc::Error> {
    sdc::Builder::new()?
        .support_scan()
        .support_central()
        .central_count(1)?
        .buffer_cfg(
            DefaultPacketPool::MTU as u16,
            DefaultPacketPool::MTU as u16,
            L2CAP_TXQ,
            L2CAP_RXQ,
        )?
        .build(p, rng, mpsl, mem)
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());

    // 1. Initialize MPSL
    let mpsl_p =
        mpsl::Peripherals::new(p.RTC0, p.TIMER0, p.TEMP, p.PPI_CH19, p.PPI_CH30, p.PPI_CH31);
    let lfclk_cfg = mpsl::raw::mpsl_clock_lfclk_cfg_t {
        source: mpsl::raw::MPSL_CLOCK_LF_SRC_RC as u8,
        rc_ctiv: mpsl::raw::MPSL_RECOMMENDED_RC_CTIV as u8,
        rc_temp_ctiv: mpsl::raw::MPSL_RECOMMENDED_RC_TEMP_CTIV as u8,
        accuracy_ppm: mpsl::raw::MPSL_DEFAULT_CLOCK_ACCURACY_PPM as u16,
        skip_wait_lfclk_started: mpsl::raw::MPSL_DEFAULT_SKIP_WAIT_LFCLK_STARTED != 0,
    };
    static MPSL: StaticCell<MultiprotocolServiceLayer> = StaticCell::new();
    let mpsl = MPSL.init(unwrap!(mpsl::MultiprotocolServiceLayer::new(
        mpsl_p, Irqs, lfclk_cfg
    )));
    spawner.spawn(unwrap!(mpsl_task(&*mpsl)));

    // 2. Configure SoftDevice Controller
    let sdc_p = sdc::Peripherals::new(
        p.PPI_CH17, p.PPI_CH18, p.PPI_CH20, p.PPI_CH21, p.PPI_CH22, p.PPI_CH23, p.PPI_CH24,
        p.PPI_CH25, p.PPI_CH26, p.PPI_CH27, p.PPI_CH28, p.PPI_CH29,
    );
    let mut rng = rng::Rng::new(p.RNG, Irqs);
    let mut sdc_mem = sdc::Mem::<4896>::new();
    let sdc = unwrap!(build_sdc(sdc_p, &mut rng, mpsl, &mut sdc_mem));

    // 3. Run application
    run(sdc).await;
}

/// Max number of connections
const CONNECTIONS_MAX: usize = 2;

/// Max number of L2CAP channels.
const L2CAP_CHANNELS_MAX: usize = 3; // Signal + att

/// Run the BLE stack.
pub async fn run<C>(controller: C)
where
    C: Controller + ControllerCmdSync<LeSetScanParams>,
{
    // Using a fixed "random" address can be useful for testing. In real scenarios, one would
    // use e.g. the MAC 6 byte array as the address (how to get that varies by the platform).
    let address: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    info!("Our address = {:?}", address);

    let mut resources: HostResources<C, DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .build();
    let runner = stack.runner();

    let printer = Printer {
        seen: RefCell::new(Deque::new()),
        hr_device: RefCell::new(Deque::new()),
    };

    let _ = join(ble_task(runner, &printer), async {
        // Phase 1: Scan for devices, use scope to drop scanner and free the stack.central()
        {
            let mut scanner = Scanner::new(stack.central());
            let mut config = ScanConfig::default();
            config.active = true;
            config.phys = PhySet::M1;
            config.interval = Duration::from_secs(1);
            config.window = Duration::from_secs(1);

            let _session = scanner.scan(&config).await.unwrap();
            Timer::after(Duration::from_secs(2)).await;
        }

        // Give the controller time to fully stop scanning
        Timer::after(Duration::from_millis(100)).await;

        info!(
            "Scan done after 2s, {:?} device(s) found",
            printer.hr_device.borrow().len()
        );

        // Phase 2: Connect to devices
        let mut central = stack.central();
        for (kind, addr) in printer.hr_device.borrow().iter() {
            info!("Connecting to {:?}", addr);
            let target = Address::new(*kind, *addr);
            let config = ConnectConfig {
                connect_params: Default::default(),
                scan_config: ScanConfig {
                    filter_accept_list: &[target],
                    ..Default::default()
                },
            };

            let conn = central.connect(&config).await.unwrap();
            info!("Connected, creating gatt client");

            let client = GattClient::<C, DefaultPacketPool, 10>::new(&stack, &conn)
                .await
                .unwrap();
            info!("gatt client created");

            let _ = join(client.task(), async {
                let service = client
                    .services_by_uuid(&Uuid::new_short(service::HEART_RATE.to_u16()))
                    .await
                    .unwrap()
                    .first()
                    .unwrap()
                    .clone();
                info!("heart rate service found");

                let c: Characteristic<u8> = client
                    .characteristic_by_uuid(
                        &service,
                        &Uuid::new_short(characteristic::HEART_RATE_MEASUREMENT.to_u16()),
                    )
                    .await
                    .unwrap();

                let mut listener = client.subscribe(&c, false).await.unwrap();
                info!("listening to heart rate measurement ... ");

                loop {
                    let data = listener.next().await;
                    info!(
                        "Got notification: {:?} (val: {})",
                        data.as_ref(),
                        data.as_ref()[0]
                    );
                }
            })
            .await;
        }
    })
    .await;
}

struct Printer {
    seen: RefCell<Deque<BdAddr, 128>>,
    hr_device: RefCell<Deque<(AddrKind, BdAddr), 8>>,
}

impl EventHandler for Printer {
    fn on_adv_reports(&self, mut it: LeAdvReportsIter<'_>) {
        let mut seen = self.seen.borrow_mut();
        let mut hr_devices = self.hr_device.borrow_mut();
        while let Some(Ok(report)) = it.next() {
            if seen.iter().find(|b| b.raw() == report.addr.raw()).is_none() {
                for ad in AdStructure::decode(report.data) {
                    match ad {
                        Ok(ad) => match ad {
                            AdStructure::CompleteServiceUuids16(ids)
                            | AdStructure::IncompleteServiceUuids16(ids) => {
                                if ids.contains(service::HEART_RATE.as_le_bytes()) {
                                    info!("new hr device: {:?}", report.addr);
                                    let _ = hr_devices.push_front((report.addr_kind, report.addr));
                                }
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

async fn ble_task<C: Controller, P: PacketPool, E: EventHandler>(
    mut runner: Runner<'_, C, P>,
    handler: &E,
) {
    loop {
        if let Err(e) = runner.run_with_handler(handler).await {
            let e = defmt::Debug2Format(&e);
            panic!("[ble_task] error: {:?}", e);
        }
    }
}
