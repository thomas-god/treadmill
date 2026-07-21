#![no_std]
#![no_main]

use core::cell::RefCell;

use bt_hci::cmd::le::LeSetScanParams;
use bt_hci::controller::ControllerCmdSync;
use defmt::{error, info, unwrap, warn};
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_futures::select::select;
use embassy_nrf::mode::Async;
use embassy_nrf::peripherals::RNG;
use embassy_nrf::{bind_interrupts, rng};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::pubsub::{DynImmediatePublisher, DynSubscriber, PubSubChannel};
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
        .support_adv()
        .support_peripheral()
        .peripheral_count(1)?
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
    let mut sdc_mem = sdc::Mem::<8192>::new();
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
    let mut runner = stack.runner();
    let handler = HRDeviceScanner {
        seen: RefCell::new(Deque::new()),
        hr_devices: RefCell::new(Deque::new()),
    };

    // Make sure the BLE stack in running in parallel of main task
    select(
        ble_task_by_ref(&mut runner, &handler),
        main(&stack, &handler),
    )
    .await;
}

async fn main<C: Controller + ControllerCmdSync<LeSetScanParams>>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    handler: &HRDeviceScanner,
) {
    let central = stack.central();

    let hr_device = scan_for_hr_device(central, handler).await.unwrap();
    info!("HR device found, connecting...");

    // Give the controller time to fully stop scanning before connecting and broadcasting
    Timer::after(Duration::from_millis(1000)).await;

    broadcast_hr_values(stack, hr_device).await;
}

struct HRDeviceScanner {
    seen: RefCell<Deque<BdAddr, 128>>,
    hr_devices: RefCell<Deque<(AddrKind, BdAddr), 8>>,
}

impl EventHandler for HRDeviceScanner {
    fn on_adv_reports(&self, mut it: LeAdvReportsIter<'_>) {
        let mut seen = self.seen.borrow_mut();
        let mut hr_devices = self.hr_devices.borrow_mut();
        while let Some(Ok(report)) = it.next() {
            if seen.iter().find(|b| b.raw() == report.addr.raw()).is_none() {
                for ad in AdStructure::decode(report.data) {
                    match ad {
                        Ok(ad) => match ad {
                            AdStructure::CompleteServiceUuids16(ids)
                            | AdStructure::IncompleteServiceUuids16(ids)
                                if ids.contains(service::HEART_RATE.as_le_bytes()) =>
                            {
                                info!("new hr device: {:?}", report.addr);
                                let _ = hr_devices.push_front((report.addr_kind, report.addr));
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

async fn ble_task_by_ref<C: Controller, P: PacketPool, E: EventHandler>(
    runner: &mut Runner<'_, C, P>,
    handler: &E,
) {
    info!("running ble task");
    loop {
        if let Err(e) = runner.run_with_handler(handler).await {
            let e = defmt::Debug2Format(&e);
            panic!("[ble_task] error: {:?}", e);
        }
    }
}

async fn scan_hr_devices_task<C, P: PacketPool>(
    central: Central<'_, C, P>,
    handler: &HRDeviceScanner,
) -> Result<(AddrKind, BdAddr), ()>
where
    C: Controller + ControllerCmdSync<LeSetScanParams>,
{
    let mut scanner = Scanner::new(central);
    let config = ScanConfig::default();

    let _session = scanner.scan(&config).await;
    Timer::after(Duration::from_secs(2)).await;

    handler.hr_devices.borrow().front().cloned().ok_or(())
}

async fn scan_for_hr_device<C, P: PacketPool>(
    central: Central<'_, C, P>,
    handler: &HRDeviceScanner,
) -> Result<(AddrKind, BdAddr), ()>
where
    C: Controller + ControllerCmdSync<LeSetScanParams>,
{
    let res = scan_hr_devices_task(central, handler).await;

    match res {
        Ok(found_device) => Ok(found_device),
        Err(err) => {
            let e = defmt::Debug2Format(&err);
            error!("[scan hr devices] error: {:?}", e);
            Err(())
        }
    }
}

async fn connect_to_hr_device<'a, C: Controller>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    mut central: Central<'a, C, DefaultPacketPool>,
    device: (AddrKind, BdAddr),
) -> GattClient<'a, C, DefaultPacketPool, 5> {
    let (kind, addr) = device;
    let target = Address::new(kind, addr);
    let config = ConnectConfig {
        connect_params: Default::default(),
        scan_config: ScanConfig {
            filter_accept_list: &[target],
            ..Default::default()
        },
    };

    let conn = central.connect(&config).await.unwrap();

    GattClient::<'a, C, DefaultPacketPool, 5>::new(stack, &conn)
        .await
        .unwrap()
}

async fn watch_hr_values_task<C: Controller, P: PacketPool>(
    client: GattClient<'_, C, P, 5>,
    publisher: DynImmediatePublisher<'_, u8>,
) {
    join(client.task(), async {
        let service = client
            .services_by_uuid(&Uuid::new_short(service::HEART_RATE.to_u16()))
            .await
            .unwrap()
            .first()
            .unwrap()
            .clone();
        info!("heart rate service found");

        let characteristic: Characteristic<u8> = client
            .characteristic_by_uuid(
                &service,
                &Uuid::new_short(characteristic::HEART_RATE_MEASUREMENT.to_u16()),
            )
            .await
            .unwrap();

        let mut listener = client.subscribe(&characteristic, false).await.unwrap();
        info!("listening to heart rate measurement ... ");

        loop {
            let data = listener.next().await;
            // TODO: hardcoded [1] ?
            publisher.publish_immediate(data.as_ref()[1]);
            info!(
                "Got notification: {:?} (val: {})",
                data.as_ref(),
                data.as_ref()[0]
            );
        }
    })
    .await;
}

async fn start_hr_server_task<C: Controller>(
    mut peripheral: Peripheral<'_, C, DefaultPacketPool>,
    mut subscriber: DynSubscriber<'_, u8>,
) {
    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "HR resynced",
        appearance: &appearance::heart_rate_sensor::GENERIC_HEART_RATE_SENSOR,
    }))
    .unwrap();
    loop {
        match advertise("HR resynced", &mut peripheral, &server).await {
            Ok(conn) => {
                let a = gatt_events_task(&conn);
                let b = notify_hr_values_task(&server, &conn, &mut subscriber);
                select(a, b).await;
            }
            Err(e) => {
                let e = defmt::Debug2Format(&e);
                panic!("[adv] error: {:?}", e);
            }
        }
    }
}

async fn broadcast_hr_values<C: Controller>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    device: (AddrKind, BdAddr),
) {
    let peripheral = stack.peripheral();
    let central = stack.central();
    let client = connect_to_hr_device(stack, central, device).await;

    let hr_channel = PubSubChannel::<NoopRawMutex, u8, 4, 4, 4>::new();
    let hr_publisher = hr_channel.dyn_immediate_publisher();
    let hr_subscriber = hr_channel.dyn_subscriber().unwrap();

    select(
        watch_hr_values_task(client, hr_publisher),
        start_hr_server_task(peripheral, hr_subscriber),
    )
    .await;
}

// GATT Server definition
#[gatt_server]
struct Server {
    hr_service: HeartRateService,
}

/// HR service
#[gatt_service(uuid = service::HEART_RATE)]
struct HeartRateService {
    #[characteristic(uuid = characteristic::HEART_RATE_MEASUREMENT, read, notify)]
    value: u8,
}

/// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
async fn advertise<'values, 'server, C: Controller>(
    name: &'values str,
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
    server: &'server Server<'values>,
) -> Result<GattConnection<'values, 'server, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut advertiser_data = [0; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::IncompleteServiceUuids16(&[service::HEART_RATE.to_le_bytes()]),
            AdStructure::CompleteLocalName(name.as_bytes()),
        ],
        &mut advertiser_data[..],
    )?;
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &advertiser_data[..len],
                scan_data: &[],
            },
        )
        .await?;
    info!("[adv] advertising");
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    info!("[adv] connection established");
    Ok(conn)
}

/// Stream Events until the connection closes.
///
/// This function will handle the GATT events and process them.
/// This is how we interact with read and write requests.
async fn gatt_events_task<P: PacketPool>(conn: &GattConnection<'_, '_, P>) -> Result<(), Error> {
    let reason = loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => break reason,
            GattConnectionEvent::Gatt { event } => {
                let reply = event.accept();
                // This step is also performed at drop(), but writing it explicitly is necessary
                // in order to ensure reply is sent.
                match reply {
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("[gatt] error sending response: {:?}", e),
                };
            }
            _ => {} // ignore other Gatt Connection Events
        }
    };
    info!("[gatt] disconnected: {:?}", reason);
    Ok(())
}

async fn notify_hr_values_task<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    subscriber: &mut DynSubscriber<'_, u8>,
) {
    let hr = server.hr_service.value;
    loop {
        let val = subscriber.next_message_pure().await;
        info!("[custom_task] new hr value {}", val);
        if hr.notify(conn, &val, true).await.is_err() {
            info!("[custom_task] error notifying connection");
            break;
        };
    }
}
