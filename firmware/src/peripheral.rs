use defmt::{error, info, warn};
use embassy_futures::select::select;
use embassy_sync::pubsub::DynSubscriber;
use trouble_host::prelude::*;

use ftms_rsc::{RscMeasurement, encode_rsc_measurement};

#[gatt_server]
struct Server {
    rsc: RSCService,
}

#[gatt_service(uuid = service::RUNNING_SPEED_AND_CADENCE)]
struct RSCService {
    #[characteristic(uuid = characteristic::RSC_MEASUREMENT, read, notify)]
    measurement: [u8; 8],

    #[characteristic(uuid = characteristic::RSC_FEATURE, read, value = ftms_rsc::encode_rsc_feature())]
    feature: [u8; 2],
}

pub async fn start_rsc_server<C: Controller>(
    mut peripheral: Peripheral<'_, C, DefaultPacketPool>,
    mut subscriber: DynSubscriber<'_, RscMeasurement>,
) {
    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "RSC service",
        appearance: &appearance::running_walking_sensor::GENERIC_RUNNING_WALKING_SENSOR,
    }))
    .expect("Unable to get RSC service server");
    loop {
        match advertise("RSC Service", &mut peripheral, &server).await {
            Ok(conn) => {
                let a = gatt_events_task(&conn);
                let b = notify_rsc_measurements(&server, &conn, &mut subscriber);
                select(a, b).await;
            }
            Err(e) => {
                let e = defmt::Debug2Format(&e);
                panic!("[adv] error: {:?}", e);
            }
            // Otherwise rust-analyzer returns a false positive claiming arm '_' is not covered.
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }
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
            AdStructure::IncompleteServiceUuids16(&[
                service::RUNNING_SPEED_AND_CADENCE.to_le_bytes()
            ]),
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
    info!("[rsc] advertising");
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    info!("[rsc] connection established");
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

async fn notify_rsc_measurements<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    subscriber: &mut DynSubscriber<'_, RscMeasurement>,
) {
    let measurement = server.rsc.measurement;
    loop {
        let val = subscriber.next_message_pure().await;
        if let Err(err) = measurement
            .notify(conn, &encode_rsc_measurement(&val), true)
            .await
        {
            error!(
                "[notify_rsc_measurements] error notifying connection: {:?}",
                err
            );
            break;
        };
    }
}
