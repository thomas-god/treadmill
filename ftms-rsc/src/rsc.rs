use crate::ftms::TreadmillData;

#[derive(Clone)]
pub struct RscMeasurement {
    pub speed_mps: f32,
    pub cadence_rpm: u8,
    pub running: bool,
    pub total_distance_m: Option<f32>,
}

const DEFAULT_CADENCE_RPM: u8 = 80;

pub fn to_rsc_measurement(data: &TreadmillData) -> Option<RscMeasurement> {
    let speed_kmh = data.instantaneous_speed?;
    let speed_mps = (speed_kmh / 3.6) as f32;

    Some(RscMeasurement {
        speed_mps,
        cadence_rpm: DEFAULT_CADENCE_RPM,
        running: true,
        total_distance_m: data.total_distance.map(|d| d as f32),
    })
}

// TODO: since distance is optional, encode with variable length.
/// Encodes to the fixed 8-byte RSC Measurement layout (no stride length, no total distance).
pub fn encode_rsc_measurement(m: &RscMeasurement) -> [u8; 8] {
    let mut buf = [0u8; 8];

    // Initial flags
    let mut flags: u8 = if m.running { 1 << 2 } else { 0 };

    // Speed
    let speed_raw = (m.speed_mps * 256.0) as u16;
    buf[1..3].copy_from_slice(&speed_raw.to_le_bytes());

    // Cadence
    buf[3] = m.cadence_rpm;

    // Optional distance
    if let Some(dist) = m.total_distance_m {
        flags |= 1 << 1; // Total Distance present
        let dist_raw = (dist * 10.0) as u32; // resolution 1/10 m
        buf[4..8].copy_from_slice(&dist_raw.to_le_bytes());
    } // else { buf[4..8] stays zeroed}

    // Flags
    buf[0] = flags;

    buf
}

pub const fn encode_rsc_feature() -> [u8; 2] {
    let flags: u16 = (1 << 1) | (1 << 2); // Total Distance + Walking/Running Status
    flags.to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_and_encodes() {
        let treadmill = TreadmillData {
            instantaneous_speed: Some(8.50),
            ..Default::default()
        };
        let rsc = to_rsc_measurement(&treadmill).unwrap();
        let bytes = encode_rsc_measurement(&rsc);
        assert_eq!(bytes.len(), 8);
        assert_eq!(bytes[0], 0b0000_0100);
    }

    #[test]
    fn no_distance_still_yields_8_bytes_zero_padded() {
        let treadmill = TreadmillData {
            instantaneous_speed: Some(8.50),
            ..Default::default()
        };
        let rsc = to_rsc_measurement(&treadmill).unwrap();
        let bytes = encode_rsc_measurement(&rsc);
        assert_eq!(bytes.len(), 8);
        assert_eq!(bytes[0] & 0b0000_0010, 0); // distance flag unset
        assert_eq!(&bytes[4..8], &[0, 0, 0, 0]);
    }
}
