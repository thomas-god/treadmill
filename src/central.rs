use defmt::{debug, info, warn};
use embassy_futures::join::join;
use embassy_sync::pubsub::DynImmediatePublisher;
use trouble_host::prelude::*;
use trouble_host::{Stack, central::Central};

use crate::rsc::RscMeasurement;
use crate::{ftms, rsc};

pub async fn watch_treadmill_data<C: Controller>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    device: (AddrKind, BdAddr),
    publisher: DynImmediatePublisher<'_, RscMeasurement>,
) {
    let central = stack.central();
    let client = connect_to_ftms_device(stack, central, device).await;

    join(client.task(), async {
        let service = client
            .services_by_uuid(&Uuid::new_short(service::FITNESS_MACHINE.to_u16()))
            .await
            .unwrap()
            .first()
            .unwrap()
            .clone();
        info!("FTMS service found");

        let characteristic: Characteristic<u8> = client
            .characteristic_by_uuid(
                &service,
                &Uuid::new_short(characteristic::TREADMILL_DATA.to_u16()),
            )
            .await
            .unwrap();

        let mut listener = client.subscribe(&characteristic, false).await.unwrap();
        info!("listening to treadmill data ... ");

        loop {
            let data = listener.next().await;
            let treadmill_data = match ftms::parse_treadmill_data(data.as_ref()) {
                Ok(data) => data,
                Err(err) => {
                    warn!("Unable to parse treadmill data: {:?}", err);
                    continue;
                }
            };
            let rsc_data = match rsc::to_rsc_measurement(&treadmill_data) {
                Some(data) => data,
                None => {
                    warn!("Unable to convert treadmill data in RSC measurement");
                    continue;
                }
            };
            debug!("RSC measurement: {:?}", &rsc_data);
            publisher.publish_immediate(rsc_data);
        }
    })
    .await;
}
async fn connect_to_ftms_device<'a, C: Controller>(
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
