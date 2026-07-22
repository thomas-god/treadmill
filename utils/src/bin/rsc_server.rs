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

// RSC Service UUID (short: 0x1814)
const RSC_SERVICE_UUID: Uuid = Uuid::from_u128(0x00001814_0000_1000_8000_00805f9b34fb);

// RSC Measurement Characteristic UUID (short: 0x2A53)
const RSC_MEASUREMENT_UUID: Uuid = Uuid::from_u128(0x00002A53_0000_1000_8000_00805f9b34fb);

// RSC Feature Characteristic UUID (short: 0x2A54)
const RSC_FEATURE_UUID: Uuid = Uuid::from_u128(0x00002A54_0000_1000_8000_00805f9b34fb);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting RSC (Running Speed and Cadence) Bluetooth Server...");

    // Create channel for peripheral events
    let (sender_tx, mut receiver_rx) = mpsc::channel::<PeripheralEvent>(256);

    // Create peripheral
    let mut peripheral = Peripheral::new(sender_tx).await?;

    // Define RSC service with characteristics
    let service = Service {
        uuid: RSC_SERVICE_UUID,
        primary: true,
        characteristics: vec![
            // RSC Feature characteristic (read-only)
            Characteristic {
                uuid: RSC_FEATURE_UUID,
                properties: vec![CharacteristicProperty::Read],
                permissions: vec![AttributePermission::Readable],
                // Features: Total Distance supported (bit 1)
                value: Some(vec![0x02, 0x00]),
                descriptors: vec![],
            },
            // RSC Measurement characteristic (notify)
            Characteristic {
                uuid: RSC_MEASUREMENT_UUID,
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
                    if request.characteristic == RSC_MEASUREMENT_UUID {
                        if subscribed {
                            println!("✓ Client subscribed to RSC measurement notifications");
                        } else {
                            println!("✗ Client unsubscribed from RSC measurement");
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

    // Add RSC service
    peripheral.add_service(&service).await?;
    println!("RSC service added");

    // Start advertising
    peripheral
        .start_advertising("RSC Sensor", &[service.uuid])
        .await?;
    println!("Advertising as 'RSC Sensor'");
    println!("Service UUID: {}", RSC_SERVICE_UUID);
    println!("Waiting for connections... Press Ctrl+C to stop.\n");

    // Main loop to update RSC data
    let cadence_spm: u8 = 80; // Fixed cadence
    let mut total_distance_m = 0.0; // total distance in meters
    let mut time = 0u16;

    loop {
        tokio::select! {
            _ = sleep(Duration::from_secs(1)) => {
                // Update dummy data with realistic running patterns
                // Speed varies between 2.0 - 3.5 m/s (7.2 - 12.6 km/h)
                let speed_ms = 2.75 + (time as f32 * 0.02).sin() * 0.75;

                // Update distance
                total_distance_m += speed_ms;
                time = time.wrapping_add(1);

                // Build RSC measurement packet
                let data = build_rsc_measurement(
                    speed_ms,
                    cadence_spm,
                    total_distance_m,
                );

                // Update the characteristic (sends notification to subscribed clients)
                if let Err(e) = peripheral
                    .update_characteristic(RSC_MEASUREMENT_UUID, data)
                    .await
                {
                    eprintln!("Error updating characteristic: {}", e);
                } else {
                    println!(
                        "🏃 Speed: {:.2} m/s ({:.1} km/h) | Cadence: {} spm | Distance: {:.1} m",
                        speed_ms,
                        speed_ms * 3.6,
                        cadence_spm,
                        total_distance_m
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

/// Build RSC measurement packet according to RSC specification
fn build_rsc_measurement(speed_ms: f32, cadence_spm: u8, total_distance_m: f32) -> Vec<u8> {
    let mut data = Vec::new();

    // Flags (1 byte)
    // Bit 0: Instantaneous Stride Length Present = 0
    // Bit 1: Total Distance Present = 1
    // Bit 2: Walking or Running Status = 1 (Running)
    // Bits 3-7: Reserved = 0
    let flags: u8 = 0b0000_0110; // Total Distance + Running
    data.push(flags);

    // Instantaneous Speed (uint16, 1/256 m/s resolution)
    let speed_raw = (speed_ms * 256.0) as u16;
    data.extend_from_slice(&speed_raw.to_le_bytes());

    // Instantaneous Cadence (uint8, steps per minute)
    data.push(cadence_spm);

    // Total Distance (uint32, 1/10 meter resolution) - only if flag bit 1 is set
    let distance_raw = (total_distance_m * 10.0) as u32;
    data.extend_from_slice(&distance_raw.to_le_bytes());

    data
}
