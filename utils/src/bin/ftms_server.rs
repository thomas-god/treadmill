use ble_peripheral_rust::{
    gatt::{
        characteristic::Characteristic,
        peripheral_event::PeripheralEvent,
        properties::{AttributePermission, CharacteristicProperty},
        service::Service,
    },
    Peripheral, PeripheralImpl,
};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use uuid::Uuid;

// FTMS Service UUID (short: 0x1826, expanded to Bluetooth Base UUID)
const FTMS_SERVICE_UUID: Uuid = Uuid::from_u128(0x00001826_0000_1000_8000_00805f9b34fb);

// Treadmill Data Characteristic UUID (short: 0x2ACD)
const TREADMILL_DATA_UUID: Uuid = Uuid::from_u128(0x00002ACD_0000_1000_8000_00805f9b34fb);

// Fitness Machine Feature Characteristic UUID (short: 0x2ACC)
const FITNESS_MACHINE_FEATURE_UUID: Uuid = Uuid::from_u128(0x00002ACC_0000_1000_8000_00805f9b34fb);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting FTMS Bluetooth Server...");

    // Create channel for peripheral events
    let (sender_tx, mut receiver_rx) = mpsc::channel::<PeripheralEvent>(256);

    // Create peripheral
    let mut peripheral = Peripheral::new(sender_tx).await?;

    // Define FTMS service with characteristics
    let service = Service {
        uuid: FTMS_SERVICE_UUID,
        primary: true,
        characteristics: vec![
            // Fitness Machine Feature characteristic (read-only)
            Characteristic {
                uuid: FITNESS_MACHINE_FEATURE_UUID,
                properties: vec![CharacteristicProperty::Read],
                permissions: vec![AttributePermission::Readable],
                value: Some(vec![0x82, 0x54, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00]),
                descriptors: vec![],
            },
            // Treadmill Data characteristic (notify)
            Characteristic {
                uuid: TREADMILL_DATA_UUID,
                properties: vec![CharacteristicProperty::Notify],
                permissions: vec![AttributePermission::Readable],
                value: None,
                descriptors: vec![],
            },
        ],
    };

    // Handle peripheral events
    tokio::spawn(async move {
        while let Some(event) = receiver_rx.recv().await {
            match event {
                PeripheralEvent::StateUpdate { is_powered } => {
                    println!("Bluetooth powered: {}", is_powered);
                }
                PeripheralEvent::CharacteristicSubscriptionUpdate {
                    request,
                    subscribed,
                } => {
                    if request.characteristic == TREADMILL_DATA_UUID {
                        if subscribed {
                            println!("✓ Client subscribed to treadmill data notifications");
                        } else {
                            println!("✗ Client unsubscribed from treadmill data");
                        }
                    }
                }
                PeripheralEvent::ReadRequest {
                    request,
                    offset: _,
                    responder,
                } => {
                    println!(
                        "Read request for characteristic: {}",
                        request.characteristic
                    );
                    // The value is already set in the characteristic definition
                    responder
                        .send(ble_peripheral_rust::gatt::peripheral_event::ReadRequestResponse {
                            value: vec![],
                            response: ble_peripheral_rust::gatt::peripheral_event::RequestResponse::Success,
                        })
                        .ok();
                }
                _ => {}
            }
        }
    });

    // Wait for Bluetooth to be powered on
    println!("Waiting for Bluetooth adapter...");
    while !peripheral.is_powered().await? {
        sleep(Duration::from_millis(100)).await;
    }
    println!("Bluetooth adapter ready");

    // Add FTMS service
    peripheral.add_service(&service).await?;
    println!("FTMS service added");

    // Start advertising
    peripheral
        .start_advertising("FTMS Treadmill", &[service.uuid])
        .await?;
    println!("Advertising as 'FTMS Treadmill'");
    println!("Service UUID: {}", FTMS_SERVICE_UUID);
    println!("Waiting for connections... Press Ctrl+C to stop.\n");

    // Main loop to update treadmill data
    let base_speed_kmh = 5.0;
    let mut distance_m = 0.0;
    let mut time = 0u16;

    loop {
        tokio::select! {
            _ = sleep(Duration::from_secs(1)) => {
                // Update dummy data
                let speed_kmh = base_speed_kmh + (time as f32 * 0.01).sin() * 2.0;
                distance_m += (speed_kmh / 3.6) * 1.0;
                time = time.wrapping_add(1);

                // Build treadmill data packet
                let data = build_treadmill_data(speed_kmh, distance_m, time);

                // Update the characteristic (sends notification to subscribed clients)
                if let Err(e) = peripheral
                    .update_characteristic(TREADMILL_DATA_UUID, data)
                    .await
                {
                    eprintln!("Error updating characteristic: {}", e);
                } else {
                    println!(
                        "📡 Speed: {:.1} km/h | Distance: {:.1} m | Time: {} s",
                        speed_kmh, distance_m, time
                    );
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down...");
                break;
            }
        }
    }

    Ok(())
}

/// Build treadmill data packet according to FTMS specification
fn build_treadmill_data(speed_kmh: f32, distance_m: f32, elapsed_time_s: u16) -> Vec<u8> {
    let mut data = Vec::new();

    // Flags (2 bytes) - indicate which fields are present
    // Bit 0: More Data = 0
    // Bit 1: Average Speed present = 0
    // Bit 2: Total Distance present = 1
    // Bit 3: Inclination and Ramp Angle present = 0
    // Bit 4: Elevation Gain present = 0
    // Bit 5: Instantaneous Pace present = 0
    // Bit 6: Average Pace present = 0
    // Bit 7: Expended Energy present = 0
    // Bit 8: Heart Rate present = 0
    // Bit 9: Metabolic Equivalent present = 0
    // Bit 10: Elapsed Time present = 1
    // Bit 11: Remaining Time present = 0
    // Bit 12-15: Reserved = 0
    let flags: u16 = 0b0000_0100_0000_0100; // Distance + Elapsed Time
    data.extend_from_slice(&flags.to_le_bytes());

    // Instantaneous Speed (uint16, 0.01 km/h resolution)
    let speed_raw = (speed_kmh * 100.0) as u16;
    data.extend_from_slice(&speed_raw.to_le_bytes());

    // Total Distance (uint24, 1 meter resolution) - only if flag is set
    let distance_raw = distance_m as u32;
    data.push((distance_raw & 0xFF) as u8);
    data.push(((distance_raw >> 8) & 0xFF) as u8);
    data.push(((distance_raw >> 16) & 0xFF) as u8);

    // Elapsed Time (uint16, 1 second resolution) - only if flag is set
    data.extend_from_slice(&elapsed_time_s.to_le_bytes());

    data
}
