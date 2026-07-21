use core::convert::TryInto;

#[derive(Debug, Default)]
pub struct TreadmillData {
    pub instantaneous_speed: Option<f64>,     // km/h
    pub average_speed: Option<f64>,           // km/h
    pub total_distance: Option<u32>,          // m
    pub inclination: Option<f64>,             // %
    pub ramp_angle_setting: Option<f64>,      // deg
    pub positive_elevation_gain: Option<f64>, // m
    pub negative_elevation_gain: Option<f64>, // m
    pub instantaneous_pace: Option<f64>,      // km/min
    pub average_pace: Option<f64>,            // km/min
    pub total_energy: Option<u16>,            // kcal
    pub energy_per_hour: Option<u16>,         // kcal
    pub energy_per_minute: Option<u8>,        // kcal
    pub heart_rate: Option<u8>,               // bpm
    pub metabolic_equivalent: Option<f64>,
    pub elapsed_time: Option<u16>,   // s
    pub remaining_time: Option<u16>, // s
    pub force_on_belt: Option<i16>,  // N
    pub power_output: Option<i16>,   // W
}

pub fn parse_treadmill_data(payload: &[u8]) -> Result<TreadmillData, &'static str> {
    let mut i = 0usize;
    let mut data = TreadmillData::default();

    let read_u16 = |p: &[u8], i: &mut usize| -> Result<u16, &'static str> {
        let bytes: [u8; 2] = p
            .get(*i..*i + 2)
            .ok_or("payload too short")?
            .try_into()
            .unwrap();
        *i += 2;
        Ok(u16::from_le_bytes(bytes))
    };
    let read_i16 = |p: &[u8], i: &mut usize| -> Result<i16, &'static str> {
        let bytes: [u8; 2] = p
            .get(*i..*i + 2)
            .ok_or("payload too short")?
            .try_into()
            .unwrap();
        *i += 2;
        Ok(i16::from_le_bytes(bytes))
    };
    let read_u8 = |p: &[u8], i: &mut usize| -> Result<u8, &'static str> {
        let b = *p.get(*i).ok_or("payload too short")?;
        *i += 1;
        Ok(b)
    };
    let read_u24 = |p: &[u8], i: &mut usize| -> Result<u32, &'static str> {
        let b = p.get(*i..*i + 3).ok_or("payload too short")?;
        *i += 3;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], 0]))
    };

    let flags = read_u16(payload, &mut i)?;

    let more_data = flags & (1 << 0) != 0;
    let avg_speed = flags & (1 << 1) != 0;
    let total_dist = flags & (1 << 2) != 0;
    let inclination = flags & (1 << 3) != 0;
    let elevation_gain = flags & (1 << 4) != 0;
    let instant_pace = flags & (1 << 5) != 0;
    let avg_pace = flags & (1 << 6) != 0;
    let energy = flags & (1 << 7) != 0;
    let heart_rate = flags & (1 << 8) != 0;
    let metabolic_eq = flags & (1 << 9) != 0;
    let elapsed_time = flags & (1 << 10) != 0;
    let remaining_time = flags & (1 << 11) != 0;
    let force_power = flags & (1 << 12) != 0;

    // bit 0 is inverted: 0 means Instantaneous Speed IS present
    if !more_data {
        data.instantaneous_speed = Some(read_u16(payload, &mut i)? as f64 * 0.01);
    }
    if avg_speed {
        data.average_speed = Some(read_u16(payload, &mut i)? as f64 * 0.01);
    }
    if total_dist {
        data.total_distance = Some(read_u24(payload, &mut i)?);
    }
    if inclination {
        data.inclination = Some(read_i16(payload, &mut i)? as f64 * 0.1);
        data.ramp_angle_setting = Some(read_i16(payload, &mut i)? as f64 * 0.1);
    }
    if elevation_gain {
        data.positive_elevation_gain = Some(read_u16(payload, &mut i)? as f64 * 0.1);
        data.negative_elevation_gain = Some(read_u16(payload, &mut i)? as f64 * 0.1);
    }
    if instant_pace {
        data.instantaneous_pace = Some(read_u8(payload, &mut i)? as f64 * 0.1);
    }
    if avg_pace {
        data.average_pace = Some(read_u8(payload, &mut i)? as f64 * 0.1);
    }
    if energy {
        data.total_energy = Some(read_u16(payload, &mut i)?);
        data.energy_per_hour = Some(read_u16(payload, &mut i)?);
        data.energy_per_minute = Some(read_u8(payload, &mut i)?);
    }
    if heart_rate {
        data.heart_rate = Some(read_u8(payload, &mut i)?);
    }
    if metabolic_eq {
        data.metabolic_equivalent = Some(read_u8(payload, &mut i)? as f64 * 0.1);
    }
    if elapsed_time {
        data.elapsed_time = Some(read_u16(payload, &mut i)?);
    }
    if remaining_time {
        data.remaining_time = Some(read_u16(payload, &mut i)?);
    }
    if force_power {
        data.force_on_belt = Some(read_i16(payload, &mut i)?);
        data.power_output = Some(read_i16(payload, &mut i)?);
    }

    Ok(data)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn parses_speed_and_distance() {
        let payload = [0x06, 0x00, 0x52, 0x03, 0x20, 0x03, 0xD2, 0x04, 0x00];
        let data = parse_treadmill_data(&payload).unwrap();
        assert_eq!(data.instantaneous_speed, Some(8.50));
        assert_eq!(data.average_speed, Some(8.00));
        assert_eq!(data.total_distance, Some(1234));
    }
}
