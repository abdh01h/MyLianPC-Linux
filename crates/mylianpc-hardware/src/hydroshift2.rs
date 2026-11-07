//! Lian Li HydroShift II LCD packet format.
//!
//! Ported from `MyLianPC.Hardware/LianLi/HydroShift2Protocol.cs`, which derives
//! from the MIT-licensed `hydroshift2-rs` and `lian-li-linux` projects. This
//! module is pure encoding/decoding: it contains no USB transport and no write
//! path. The ring-frame uploader (TinyUZ) is not ported yet.

use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use chrono::{DateTime, Datelike, Local, Timelike, Utc};
use serde::{Deserialize, Serialize};

pub const VID: u16 = 0x1CBE;
pub const PID_CIRCLE: u16 = 0xA021;
pub const PID_SQUARE: u16 = 0xA034;
pub const OUT_ENDPOINT: u8 = 0x01;
pub const IN_ENDPOINT: u8 = 0x81;
pub const WIDTH: u32 = 480;
pub const HEIGHT: u32 = 480;
pub const RING_LED_COUNT: usize = 24;
pub const PUMP_MIN_RPM: u16 = 1600;
pub const PUMP_MAX_RPM_CIRCLE: u16 = 2500;
pub const HEADER_SIZE: usize = 512;
pub const MAX_RING_FRAMES: usize = 48;
pub const FAN_LED_COUNT: usize = 24;
/// Largest uncompressed animation we upload: keeps the data inside the
/// firmware's 4 KB decode cache.
pub const MAX_RAW_LIGHTING_BYTES: usize = 4096;

const PLAIN_SIZE: usize = 500;
const ENCRYPTED_SIZE: usize = 504;
const KEY: &[u8; 8] = b"slv3tuzx";

type Encryptor = cbc::Encryptor<des::Des>;
type Decryptor = cbc::Decryptor<des::Des>;

pub mod cmd {
    pub const GET_VERSION: u8 = 0x0A;
    pub const ROTATION: u8 = 0x0D;
    pub const BRIGHTNESS: u8 = 0x0E;
    pub const FRAME_RATE: u8 = 0x0F;
    pub const SET_CLOCK: u8 = 0x33;
    pub const STOP_CLOCK: u8 = 0x34;
    pub const JPEG: u8 = 0x65;
    pub const PNG_OVERLAY: u8 = 0x66;
    pub const STOP_PLAY: u8 = 0x7B;
    pub const GET_PARAMS: u8 = 0xFA;
    pub const SYNC_PUMP_FAN: u8 = 0xFB;
    pub const PUSH_RGB: u8 = 0xFC;
}

/// Millisecond timestamp the firmware expects: ms elapsed since local midnight
/// of the previous day (L-Connect compatible).
pub fn timestamp(now: DateTime<Local>) -> u32 {
    let naive = now.naive_local();
    let yesterday = naive
        .date()
        .pred_opt()
        .unwrap_or(naive.date())
        .and_hms_opt(0, 0, 0)
        .unwrap_or(naive);
    (naive - yesterday).num_milliseconds().max(0) as u32
}

/// Builds the 512-byte encrypted header: DES-CBC/PKCS7 over 500 plaintext
/// bytes, then zero padding and the `A1 1A` trailer.
pub fn header(command: u8, parameters: &[u8], timestamp: u32) -> [u8; HEADER_SIZE] {
    assert!(
        parameters.len() <= PLAIN_SIZE - 8,
        "parameters exceed 492 bytes"
    );

    let mut plain = [0u8; PLAIN_SIZE];
    plain[0] = command;
    plain[2] = 0x1A;
    plain[3] = 0x6D;
    plain[4..8].copy_from_slice(&timestamp.to_le_bytes());
    plain[8..8 + parameters.len()].copy_from_slice(parameters);

    let encrypted = Encryptor::new_from_slices(KEY, KEY)
        .expect("8-byte DES key and IV")
        .encrypt_padded_vec_mut::<Pkcs7>(&plain);
    debug_assert_eq!(encrypted.len(), ENCRYPTED_SIZE);

    let mut packet = [0u8; HEADER_SIZE];
    packet[..ENCRYPTED_SIZE].copy_from_slice(&encrypted);
    packet[510] = 0xA1;
    packet[511] = 0x1A;
    packet
}

/// Inverse of [`header`]; used by tests and diagnostics.
pub fn decrypt_header(packet: &[u8]) -> Option<Vec<u8>> {
    if packet.len() < ENCRYPTED_SIZE {
        return None;
    }
    Decryptor::new_from_slices(KEY, KEY)
        .ok()?
        .decrypt_padded_vec_mut::<Pkcs7>(&packet[..ENCRYPTED_SIZE])
        .ok()
}

pub fn command(command: u8, parameters: &[u8]) -> [u8; HEADER_SIZE] {
    header(command, parameters, timestamp(Local::now()))
}

/// Header + JPEG (background, 0x65) or PNG (transparent overlay, 0x66) in one
/// transfer. Size is big-endian at `p[8..12]`.
pub fn image(data: &[u8], png: bool, timestamp: Option<u32>) -> Vec<u8> {
    let parameters = (data.len() as u32).to_be_bytes();
    let header = header(
        if png { cmd::PNG_OVERLAY } else { cmd::JPEG },
        &parameters,
        timestamp.unwrap_or_else(|| self::timestamp(Local::now())),
    );
    let mut packet = Vec::with_capacity(HEADER_SIZE + data.len());
    packet.extend_from_slice(&header);
    packet.extend_from_slice(data);
    packet
}

pub fn clock(utc: DateTime<Utc>) -> [u8; HEADER_SIZE] {
    let parameters = [
        (utc.year() >> 8) as u8,
        utc.year() as u8,
        utc.month() as u8,
        utc.day() as u8,
        utc.hour() as u8,
        utc.minute() as u8,
        utc.second() as u8,
        2,
    ];
    command(cmd::SET_CLOCK, &parameters)
}

/// CRC-16/XMODEM (poly 0x1021, init 0).
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for byte in data {
        crc ^= (*byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Maps a pump RPM target to the firmware's inverse control value (Circle
/// model). Lower value = faster pump.
pub fn pump_control_value(rpm: i32) -> u16 {
    let r = rpm.clamp(PUMP_MIN_RPM as i32, PUMP_MAX_RPM_CIRCLE as i32);
    let value = if r <= 1720 {
        1500 - ((r - 1600) as f64 * 1.625) as i32
    } else if r <= 1870 {
        1300 - (r - 1720) * 2
    } else if r <= 2000 {
        1000 - ((r - 1870) as f64 * 1.23) as i32
    } else if r <= 2300 {
        840 - (r - 2000) * 2
    } else if r <= 2400 {
        240 - ((r - 2300) as f64 * 1.8) as i32
    } else {
        60 - ((r - 2400) as f64 * 0.5) as i32
    };
    value.max(0) as u16
}

/// SyncPumpFan control block: `FF 0F A2 00`, pump value (BE), three raw fan
/// duties, CRC16 over bytes 0..12 at 12..14 (BE).
pub fn sync_pump_fan_parameters(pump_rpm: i32, fan1: u8, fan2: u8, fan3: u8) -> [u8; 16] {
    let mut control = [0u8; 16];
    control[0] = 0xFF;
    control[1] = 0x0F;
    control[2] = 0xA2;
    control[4..6].copy_from_slice(&pump_control_value(pump_rpm).to_be_bytes());
    control[6] = fan1;
    control[7] = fan2;
    control[8] = fan3;
    let crc = crc16(&control[..12]);
    control[12..14].copy_from_slice(&crc.to_be_bytes());
    control
}

/// PushRgbData (0xFC): TinyUZ(raw frames of RGB triplets) + frame count (u16
/// BE) + interval ticks + LED count. A frame is the 24 ring LEDs, optionally
/// followed by 24 LEDs per fan wired to the pump head. The device loops the
/// animation by itself.
pub fn ring_frames(
    frames: &[Vec<u8>],
    interval_ticks: u8,
    timestamp: Option<u32>,
) -> Result<Vec<u8>, String> {
    if frames.is_empty() || frames.len() > MAX_RING_FRAMES {
        return Err(format!("ring animations need 1-{MAX_RING_FRAMES} frames"));
    }
    let frame_bytes = frames[0].len();
    let leds = frame_bytes / 3;
    if frame_bytes % 3 != 0
        || leds < RING_LED_COUNT
        || (leds - RING_LED_COUNT) % FAN_LED_COUNT != 0
        || leds > RING_LED_COUNT + 4 * FAN_LED_COUNT
    {
        return Err(format!(
            "each frame must be {RING_LED_COUNT} ring LEDs plus {FAN_LED_COUNT} per fan (0-4 fans)"
        ));
    }
    if frames.len() * frame_bytes > MAX_RAW_LIGHTING_BYTES {
        return Err(format!(
            "the animation is larger than the cooler's {MAX_RAW_LIGHTING_BYTES}-byte decode cache"
        ));
    }

    let mut raw = Vec::with_capacity(frames.len() * frame_bytes);
    for frame in frames {
        if frame.len() != frame_bytes {
            return Err("all frames must have the same length".to_string());
        }
        raw.extend_from_slice(frame);
    }

    let compressed = crate::tinyuz::compress(&raw);
    let round_trip = crate::tinyuz::decompress(&compressed, raw.len())
        .map_err(|error| error.to_string())?;
    if round_trip != raw {
        return Err("TinyUZ round-trip verification failed".to_string());
    }

    let mut payload = Vec::with_capacity(compressed.len() + 4);
    payload.extend_from_slice(&compressed);
    payload.extend_from_slice(&(frames.len() as u16).to_be_bytes());
    payload.push(interval_ticks);
    payload.push(leds as u8);

    let mut parameters = [0u8; 8];
    parameters[4..8].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    let header = header(
        cmd::PUSH_RGB,
        &parameters,
        timestamp.unwrap_or_else(|| self::timestamp(Local::now())),
    );

    let mut packet = Vec::with_capacity(HEADER_SIZE + payload.len());
    packet.extend_from_slice(&header);
    packet.extend_from_slice(&payload);
    Ok(packet)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Telemetry {
    pub coolant_c: i32,
    pub fan_rpm: [u16; 3],
    pub pump_rpm: u16,
    pub mac: String,
    pub lighting_fan_count: Option<u8>,
}

/// Parses a GetParams (0xFA) reply. Byte 9 is the number of fans whose lighting
/// the pump head drives when byte 8 carries the 0x0A marker.
pub fn parse_telemetry(reply: &[u8]) -> Option<Telemetry> {
    if reply.len() < 28 {
        return None;
    }
    Some(Telemetry {
        coolant_c: reply[13] as i32,
        fan_rpm: [
            u16::from_be_bytes([reply[14], reply[15]]),
            u16::from_be_bytes([reply[16], reply[17]]),
            u16::from_be_bytes([reply[18], reply[19]]),
        ],
        pump_rpm: u16::from_be_bytes([reply[20], reply[21]]),
        mac: reply[22..28]
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect(),
        lighting_fan_count: if reply[8] == 0x0A && reply[9] <= 4 {
            Some(reply[9])
        } else {
            None
        },
    })
}

/// Firmware version string from a GetVersion (0x0A) reply (bytes 8..40, NUL
/// terminated).
pub fn parse_version(reply: &[u8]) -> Option<String> {
    if reply.len() < 40 {
        return None;
    }
    let text = String::from_utf8_lossy(&reply[8..40])
        .split('\0')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn header_is_512_bytes_with_trailer_and_decrypts_to_expected_plaintext() {
        let header = self::header(cmd::JPEG, &[0, 1, 0x23, 0x45], 100_001);
        assert_eq!(header.len(), 512);
        assert_eq!(&header[504..], &[0, 0, 0, 0, 0, 0, 0xA1, 0x1A]);

        let plain = decrypt_header(&header).expect("decrypts");
        let mut expected = [0u8; PLAIN_SIZE];
        expected[0] = cmd::JPEG;
        expected[2] = 0x1A;
        expected[3] = 0x6D;
        expected[4..8].copy_from_slice(&100_001u32.to_le_bytes());
        expected[8] = 0;
        expected[9] = 1;
        expected[10] = 0x23;
        expected[11] = 0x45;
        assert_eq!(plain, expected);
    }

    #[test]
    fn image_packet_carries_big_endian_length_then_payload() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xD9];
        let packet = image(&jpeg, false, Some(1));
        assert_eq!(packet.len(), HEADER_SIZE + jpeg.len());
        assert_eq!(&packet[HEADER_SIZE..], &jpeg);

        let plain = decrypt_header(&packet).expect("decrypts");
        assert_eq!(plain[0], cmd::JPEG);
        assert_eq!(u32::from_be_bytes([plain[8], plain[9], plain[10], plain[11]]), 4);
    }

    #[test]
    fn crc16_matches_reference_vectors() {
        assert_eq!(crc16(&[]), 0x0000);
        assert_eq!(crc16(&[0x01]), 0x1021);
        assert_eq!(crc16(b"123456789"), 0x31C3);
    }

    #[test]
    fn pump_mapping_matches_reference() {
        for (rpm, expected) in [
            (1000, 1500),
            (1600, 1500),
            (2000, 841),
            (2001, 838),
            (2500, 10),
            (9999, 10),
        ] {
            assert_eq!(pump_control_value(rpm), expected, "rpm {rpm}");
        }
    }

    #[test]
    fn sync_pump_fan_block_layout_and_crc() {
        let control = sync_pump_fan_parameters(2000, 10, 20, 30);
        assert_eq!(&control[..4], &[0xFF, 0x0F, 0xA2, 0x00]);
        assert_eq!(u16::from_be_bytes([control[4], control[5]]), 841);
        assert_eq!(&control[6..9], &[10, 20, 30]);
        assert_eq!(
            u16::from_be_bytes([control[12], control[13]]),
            crc16(&control[..12])
        );
    }

    #[test]
    fn parses_telemetry_reply() {
        let mut reply = [0u8; 512];
        reply[0] = cmd::GET_PARAMS;
        reply[13] = 31;
        reply[14..16].copy_from_slice(&1200u16.to_be_bytes());
        reply[16..18].copy_from_slice(&1210u16.to_be_bytes());
        reply[18..20].copy_from_slice(&1190u16.to_be_bytes());
        reply[20..22].copy_from_slice(&2450u16.to_be_bytes());

        let telemetry = parse_telemetry(&reply).expect("parses");
        assert_eq!(telemetry.coolant_c, 31);
        assert_eq!(telemetry.fan_rpm, [1200, 1210, 1190]);
        assert_eq!(telemetry.pump_rpm, 2450);
        assert_eq!(telemetry.lighting_fan_count, None);
        assert!(parse_telemetry(&[0u8; 10]).is_none());

        reply[8] = 0x0A;
        reply[9] = 3;
        assert_eq!(parse_telemetry(&reply).unwrap().lighting_fan_count, Some(3));
    }

    #[test]
    fn parses_firmware_version() {
        let mut reply = [0u8; 512];
        reply[8..8 + 18].copy_from_slice(b"lianlih2_0003_0020");
        assert_eq!(
            parse_version(&reply).as_deref(),
            Some("lianlih2_0003_0020")
        );
    }

    #[test]
    fn timestamp_is_milliseconds_since_previous_midnight() {
        let noon = Local
            .with_ymd_and_hms(2026, 11, 7, 12, 0, 0)
            .single()
            .expect("valid local time");
        assert_eq!(timestamp(noon), 36 * 60 * 60 * 1000);
    }

    #[test]
    fn ring_packet_has_header_compressed_payload_and_trailer_fields() {
        let frame = [255u8, 0, 0].repeat(RING_LED_COUNT);
        let packet = ring_frames(std::slice::from_ref(&frame), 50, Some(1)).expect("builds");
        let plain = decrypt_header(&packet).expect("decrypts");
        assert_eq!(plain[0], cmd::PUSH_RGB);

        let payload_len = u32::from_be_bytes([plain[12], plain[13], plain[14], plain[15]]) as usize;
        assert_eq!(payload_len, packet.len() - HEADER_SIZE);

        let payload = &packet[HEADER_SIZE..];
        assert_eq!(
            u16::from_be_bytes([payload[payload.len() - 4], payload[payload.len() - 3]]),
            1
        );
        assert_eq!(payload[payload.len() - 2], 50);
        assert_eq!(payload[payload.len() - 1], RING_LED_COUNT as u8);

        let raw = crate::tinyuz::decompress(&payload[..payload.len() - 4], frame.len())
            .expect("decodes");
        assert_eq!(raw, frame);
    }

    #[test]
    fn ring_rejects_too_many_frames() {
        let frame = vec![0u8; RING_LED_COUNT * 3];
        let frames = vec![frame; MAX_RING_FRAMES + 1];
        assert!(ring_frames(&frames, 50, Some(1)).is_err());
    }

    #[test]
    fn ring_with_three_fans_declares_96_leds() {
        let frame = vec![0u8; (RING_LED_COUNT + 3 * FAN_LED_COUNT) * 3];
        let packet = ring_frames(&[frame.clone(), frame], 40, Some(1)).expect("builds");
        let payload = &packet[HEADER_SIZE..];
        assert_eq!(payload[payload.len() - 1], 96);
        assert_eq!(
            u16::from_be_bytes([payload[payload.len() - 4], payload[payload.len() - 3]]),
            2
        );
    }

    #[test]
    fn ring_rejects_partial_fans_and_oversized_animations() {
        let partial = vec![0u8; (RING_LED_COUNT + 10) * 3];
        assert!(ring_frames(&[partial], 40, Some(1)).is_err());

        let three_fans = vec![0u8; 96 * 3];
        let frames = vec![three_fans; 15];
        assert!(ring_frames(&frames, 40, Some(1)).is_err());
    }
}
