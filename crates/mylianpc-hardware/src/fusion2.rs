//! GIGABYTE Fusion2 / IT5711 lighting controller (read-only info query).
//!
//! Ported from `MyLianPC.Hardware/Gigabyte/Fusion2Controller.cs`, which itself
//! derives from OpenRGB's GPL-2.0-or-later implementation. This module only
//! issues the identification queries (`0x60`, `0x61`); it contains no effect,
//! colour or flash write.

use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use mylianpc_core::color::RgbColor;
use serde::{Deserialize, Serialize};

use crate::hidraw::{has_usage, HidRaw};
use crate::inventory;

const VID: &str = "048d";
const PID: &str = "5711";
const REPORT_ID: u8 = 0xCC;
const PACKET_SIZE: usize = 64;
const VENDOR_USAGE_PAGE: u16 = 0xFF89;
const VENDOR_USAGE: u16 = 0x00CC;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fusion2Info {
    pub product_id: u16,
    pub device_num: u8,
    pub strip_detect: u8,
    pub firmware: String,
    pub support_flags: u8,
    pub description: String,
    pub chip_id: u32,
    /// Per-header byte order, as reported by the controller.
    pub byte_order: [u32; 6],
    pub hidraw: String,
}

/// Finds the vendor HID collection: VID/PID `048d:5711`, descriptor usage page
/// `0xFF89`, usage `0xCC`. On this board that is `/dev/hidraw8`.
pub fn find_vendor_hidraw() -> Result<HidRaw> {
    let mut candidates = Vec::new();

    for device in inventory::hidraw_devices() {
        if !device.vid.as_deref().unwrap_or_default().eq_ignore_ascii_case(VID)
            || !device.pid.as_deref().unwrap_or_default().eq_ignore_ascii_case(PID)
        {
            continue;
        }

        let hid = match HidRaw::open(&device.node) {
            Ok(hid) => hid,
            Err(error) => {
                candidates.push(format!("{}: {error}", device.node));
                continue;
            }
        };

        match hid.report_descriptor() {
            Ok(descriptor) if has_usage(&descriptor, VENDOR_USAGE_PAGE, VENDOR_USAGE) => {
                return Ok(hid);
            }
            Ok(_) => candidates.push(format!("{}: vendor usage not present", device.node)),
            Err(error) => candidates.push(format!("{}: descriptor read failed: {error}", device.node)),
        }
    }

    if candidates.is_empty() {
        bail!("no hidraw device with VID:PID {VID}:{PID} was found");
    }
    bail!(
        "Fusion2 vendor collection not found ({})",
        candidates.join("; ")
    )
}

/// Reads the identification block. This is a query, not a lighting change:
/// `0x60` returns device/firmware/description/chip id, and `0x61` (on the
/// IT5711) returns the remaining header byte orders.
pub fn query_info(hid: &HidRaw, product_id: u16) -> Result<Fusion2Info> {
    let info = query(hid, 0x60).context("querying 0x60")?;
    let device_num = info[2];
    let strip_detect = info[3];
    let firmware_raw = le_u32(&info[4..8]);
    let firmware = format!(
        "{}.{}.{}.{}",
        firmware_raw & 0xFF,
        (firmware_raw >> 8) & 0xFF,
        (firmware_raw >> 16) & 0xFF,
        (firmware_raw >> 24) & 0xFF
    );
    let support_flags = info[11];
    let description = String::from_utf8_lossy(&info[12..40])
        .split('\0')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();

    let mut byte_order = [0u32; 6];
    byte_order[0] = le_u32(&info[44..48]);
    byte_order[1] = le_u32(&info[48..52]);
    let chip_id = le_u32(&info[56..60]);

    if product_id == 0x5711 {
        let extension = query(hid, 0x61).context("querying 0x61")?;
        for index in 0..4 {
            let offset = 4 + index * 4;
            byte_order[2 + index] = le_u32(&extension[offset..offset + 4]);
        }
    }

    Ok(Fusion2Info {
        product_id,
        device_num,
        strip_detect,
        firmware,
        support_flags,
        description,
        chip_id,
        byte_order,
        hidraw: hid.path().display().to_string(),
    })
}

fn query(hid: &HidRaw, command: u8) -> Result<Vec<u8>> {
    let mut packet = [0u8; PACKET_SIZE];
    packet[0] = REPORT_ID;
    packet[1] = command;
    hid.set_feature(&packet)?;

    let reply = hid.get_feature(REPORT_ID, PACKET_SIZE)?;
    if reply.len() < PACKET_SIZE {
        bail!(
            "short feature reply for 0x{command:02X}: {} bytes",
            reply.len()
        );
    }
    if reply[0] != REPORT_ID {
        bail!(
            "unexpected report id 0x{:02X} for 0x{command:02X}",
            reply[0]
        );
    }
    Ok(reply)
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Hardware effects understood by the Fusion2 controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Fusion2Effect {
    Static = 1,
    Pulse = 2,
    Blinking = 3,
    ColorCycle = 4,
    Wave = 6,
    Random = 8,
}

/// A lighting zone on the controller: an addressable header, or a single
/// onboard LED group.
#[derive(Clone, Debug)]
pub struct Fusion2Zone {
    pub id: &'static str,
    pub name: &'static str,
    pub led_index: u32,
    pub addressable: bool,
    pub direct_header: u8,
    pub strip_slot: i32,
    pub builtin_effect_bit: u8,
}

/// Zones of the B850 AORUS ELITE WIFI7 (OpenRGB layout `it5711_9_device`).
pub const B850_ZONES: [Fusion2Zone; 5] = [
    Fusion2Zone {
        id: "fusion2:argb1",
        name: "ARGB_V2_1 header",
        led_index: 5,
        addressable: true,
        direct_header: 0x58,
        strip_slot: 0,
        builtin_effect_bit: 0x01,
    },
    Fusion2Zone {
        id: "fusion2:argb2",
        name: "ARGB_V2_2 header",
        led_index: 6,
        addressable: true,
        direct_header: 0x59,
        strip_slot: 1,
        builtin_effect_bit: 0x02,
    },
    Fusion2Zone {
        id: "fusion2:argb3",
        name: "ARGB_V2_3 header",
        led_index: 7,
        addressable: true,
        direct_header: 0x62,
        strip_slot: 2,
        builtin_effect_bit: 0x08,
    },
    Fusion2Zone {
        id: "fusion2:chipset",
        name: "Chipset accent",
        led_index: 2,
        addressable: false,
        direct_header: 0,
        strip_slot: -1,
        builtin_effect_bit: 0,
    },
    Fusion2Zone {
        id: "fusion2:ledc",
        name: "LED_C",
        led_index: 4,
        addressable: false,
        direct_header: 0,
        strip_slot: -1,
        builtin_effect_bit: 0,
    },
];

/// Hardware effect on one zone. Speed 0 (slowest) .. 100 (fastest);
/// brightness 0-100. Send [`encode_apply`] afterwards once for a batch.
pub fn encode_effect(
    zone: &Fusion2Zone,
    effect: Fusion2Effect,
    color: RgbColor,
    brightness_percent: i32,
    speed_percent: i32,
    random: bool,
) -> [u8; PACKET_SIZE] {
    let mut packet = [0u8; PACKET_SIZE];
    packet[0] = REPORT_ID;

    let led = zone.led_index;
    packet[1] = if led < 8 {
        (0x20 + led) as u8
    } else {
        (0x90 + (led - 8)) as u8
    };
    packet[2..6].copy_from_slice(&(1u32 << led).to_le_bytes());
    packet[11] = effect as u8;

    // The controller misbehaves with pulse brightness above 100 (OpenRGB note),
    // so pulse is scaled to 0-100.
    let max_brightness = if effect == Fusion2Effect::Pulse { 100 } else { 255 };
    packet[12] = (brightness_percent.clamp(0, 100) * max_brightness / 100) as u8;
    packet[13] = 0;
    let color_value = color.b as u32 | (color.g as u32) << 8 | (color.r as u32) << 16;
    packet[14..18].copy_from_slice(&color_value.to_le_bytes());

    // Controller: 0 = fast, 9 = slow.
    let speed = 9 - (speed_percent.clamp(0, 100) as f64 * 9.0 / 100.0).round() as i32;
    let (mut p0, mut p1, mut p2) = (0u16, 0u16, 0u16);
    let (mut e0, mut e1) = (if random { 7u8 } else { 0u8 }, 0u8);

    match effect {
        Fusion2Effect::Pulse => {
            p0 = if speed <= 6 {
                (400 + speed * 100) as u16
            } else {
                (1000 + (speed - 6) * 200) as u16
            };
            p1 = p0;
            p2 = 200;
        }
        Fusion2Effect::Blinking => {
            p0 = 100;
            p1 = 100;
            p2 = (speed * 200 + 700) as u16;
        }
        Fusion2Effect::ColorCycle => {
            p0 = (speed * 100 + 300 + if speed > 8 { 1300 * (speed - 8) } else { 0 }) as u16;
            p1 = (p0 as i32 - 200) as u16;
            e0 = 7;
        }
        Fusion2Effect::Wave => {
            let step = speed + 1;
            p0 = ((step * step + step + 10) * 5 / 2) as u16;
            e0 = 7;
            e1 = 1;
        }
        Fusion2Effect::Random => {
            p0 = 100;
            e0 = 1;
            e1 = 5;
        }
        Fusion2Effect::Static => {}
    }

    packet[22..24].copy_from_slice(&p0.to_le_bytes());
    packet[24..26].copy_from_slice(&p1.to_le_bytes());
    packet[26..28].copy_from_slice(&p2.to_le_bytes());
    packet[30] = e0;
    packet[31] = e1;
    packet
}

/// Commits all pending effect packets.
pub fn encode_apply(product_id: u16) -> [u8; PACKET_SIZE] {
    let mut packet = [0u8; PACKET_SIZE];
    packet[0] = REPORT_ID;
    packet[1] = 0x28;
    packet[2] = 0xFF;
    packet[3] = if product_id == 0x5711 { 0x07 } else { 0x00 };
    packet
}

/// Per-LED colours on an addressable header (direct mode), split into the
/// controller's 19-LED packets. This disables the header's built-in effect
/// engine on the device when sent; this function only builds the packets.
pub fn encode_direct(
    zone: &Fusion2Zone,
    colors: &[RgbColor],
    byte_order: u32,
) -> Vec<[u8; PACKET_SIZE]> {
    if !zone.addressable {
        return Vec::new();
    }

    let (mut bo_r, mut bo_g, mut bo_b) = (
        ((byte_order >> 16) & 0xFF) as usize,
        ((byte_order >> 8) & 0xFF) as usize,
        (byte_order & 0xFF) as usize,
    );
    if byte_order == 0 || bo_r > 2 || bo_g > 2 || bo_b > 2 {
        // GRB, the WS2812 default.
        (bo_r, bo_g, bo_b) = (1, 0, 2);
    }

    let mut packets = Vec::new();
    let mut sent = 0usize;
    for start in (0..colors.len()).step_by(19) {
        let count = (colors.len() - start).min(19);
        let mut packet = [0u8; PACKET_SIZE];
        packet[0] = REPORT_ID;
        packet[1] = zone.direct_header;
        packet[2..4].copy_from_slice(&(sent as u16).to_le_bytes());
        packet[4] = (count * 3) as u8;
        for index in 0..count {
            let color = colors[start + index];
            let offset = 5 + index * 3;
            packet[offset + bo_r] = color.r;
            packet[offset + bo_g] = color.g;
            packet[offset + bo_b] = color.b;
        }
        sent += count * 3;
        packets.push(packet);
    }
    packets
}

/// Reset sequence: clears the effect registers for the headers (and the IT5711
/// Gen2 registers), then commits.
pub fn encode_reset(product_id: u16) -> Vec<[u8; PACKET_SIZE]> {
    let mut packets = Vec::new();
    for register in 0x20..=0x27u8 {
        packets.push(simple(register, 0, 0, 0));
    }
    if product_id == 0x5711 {
        for register in 0x90..=0x92u8 {
            packets.push(simple(register, 0, 0, 0));
        }
    }
    packets.push(encode_apply(product_id));
    packets
}

/// Save the current hardware effects to the controller's flash so they survive
/// power-off with no software running.
pub fn encode_save_flash() -> [[u8; PACKET_SIZE]; 2] {
    [simple(0x47, 1, 0, 0), simple(0x5E, 0, 0, 0)]
}

fn simple(a: u8, b: u8, c: u8, d: u8) -> [u8; PACKET_SIZE] {
    let mut packet = [0u8; PACKET_SIZE];
    packet[0] = REPORT_ID;
    packet[1] = a;
    packet[2] = b;
    packet[3] = c;
    packet[4] = d;
    packet
}

/// An open Fusion2 controller.
///
/// Ported from `MyLianPC.Hardware/Gigabyte/Fusion2Controller.cs`. The packet
/// builders are pure; this type owns the HID handle and performs the writes.
pub struct Fusion2Controller {
    hid: HidRaw,
    info: Fusion2Info,
    byte_order: [u32; 6],
    builtin_disabled_mask: i32,
    direct_active: bool,
}

impl Fusion2Controller {
    /// Finds and opens the vendor HID collection and reads its identification.
    pub fn open() -> Result<Self> {
        let hid = find_vendor_hidraw()?;
        let info = query_info(&hid, 0x5711)?;
        let byte_order = info.byte_order;
        Ok(Self {
            hid,
            info,
            byte_order,
            builtin_disabled_mask: -1,
            direct_active: false,
        })
    }

    pub fn info(&self) -> &Fusion2Info {
        &self.info
    }

    pub fn hidraw_path(&self) -> &str {
        &self.info.hidraw
    }

    pub fn supports_gen2(&self) -> bool {
        self.info.device_num == 0
            && (self.info.support_flags & 0x01) != 0
            && self.info.strip_detect == 1
    }

    pub fn direct_active(&self) -> bool {
        self.direct_active
    }

    /// Puts the controller into a known state: LampArray and audio-beat mode
    /// off, effect registers cleared.
    pub fn initialize(&mut self) -> Result<()> {
        self.builtin_disabled_mask = -1;
        if self.info.support_flags & 0x02 != 0 {
            self.send(0x48, 0, 0, 0)?; // disable HID LampArray mode
        }
        self.reset()?;
        self.send(0x31, 0, 0, 0)?; // disable beat (audio sync) mode
        Ok(())
    }

    /// Re-takes control without the visible reset of [`Self::initialize`].
    pub fn reassert(&mut self) -> Result<()> {
        self.builtin_disabled_mask = -1;
        if self.info.support_flags & 0x02 != 0 {
            self.send(0x48, 0, 0, 0)?;
        }
        self.send(0x31, 0, 0, 0)?;
        Ok(())
    }

    /// Re-arms the headers' effect engine once before a batch of zone updates.
    pub fn begin_batch(&mut self) -> Result<()> {
        self.builtin_disabled_mask = 0;
        self.send(0x32, 0, 0, 0)?;
        thread::sleep(Duration::from_millis(50));
        Ok(())
    }

    /// Hardware effect on one zone. Call [`Self::apply`] once afterwards.
    pub fn set_effect(
        &mut self,
        zone: &Fusion2Zone,
        effect: Fusion2Effect,
        color: RgbColor,
        brightness_percent: i32,
        speed_percent: i32,
        random: bool,
    ) -> Result<()> {
        if zone.addressable {
            self.set_builtin_effects(zone, true)?;
        }
        let packet = encode_effect(
            zone,
            effect,
            color,
            brightness_percent,
            speed_percent,
            random,
        );
        self.hid.set_feature(&packet)?;
        Ok(())
    }

    /// Per-LED colours on an addressable header. Not stored in flash; the
    /// service re-applies it after boot.
    pub fn set_direct(&mut self, zone: &Fusion2Zone, colors: &[RgbColor]) -> Result<()> {
        if !zone.addressable {
            bail!("direct mode needs an addressable header");
        }
        self.set_builtin_effects(zone, false)?;
        let slot = zone.strip_slot;
        let order = if (0..6).contains(&slot) {
            self.byte_order[slot as usize]
        } else {
            0
        };
        let packets = encode_direct(zone, colors, order);
        for packet in packets {
            self.hid.set_feature(&packet)?;
        }
        self.direct_active = true;
        Ok(())
    }

    /// Commits all pending effect packets.
    pub fn apply(&mut self) -> Result<()> {
        self.send(
            0x28,
            0xFF,
            if self.info.product_id == 0x5711 { 0x07 } else { 0x00 },
            0,
        )
    }

    /// Saves the current hardware effects to the controller's flash.
    pub fn save_to_flash(&mut self) -> Result<()> {
        self.send(0x47, 1, 0, 0)?;
        thread::sleep(Duration::from_millis(20));
        self.send(0x5E, 0, 0, 0)?;
        Ok(())
    }

    /// Asks a Gen2-capable header how many LEDs the attached strip reports.
    /// Returns `None` when detection is unsupported and `Some(0)` when the
    /// strip does not report itself.
    pub fn detect_strip_leds(&mut self, slot: i32) -> Result<Option<usize>> {
        if !self.supports_gen2() || !(0..4).contains(&slot) {
            return Ok(None);
        }
        let delta = [4u8, 5, 0, 1];
        let scan = 0x38u8 + delta[slot as usize];
        self.send(scan, 0, 0, 0)?;
        thread::sleep(Duration::from_millis(700));
        self.send(scan + 2, 0, 0, 0)?;
        let reply = self.query()?;
        let segments = reply[1].min(15) as usize;
        let mut total = 0usize;
        for index in 0..segments {
            let offset = 2 + index * 2;
            total += u16::from_le_bytes([reply[offset], reply[offset + 1]]) as usize;
        }
        self.send(0x34, 0, 0, 0)?; // restore default LED-count buckets
        thread::sleep(Duration::from_millis(20));
        self.send(0x47, 0, 0, 0)?;
        thread::sleep(Duration::from_millis(20));
        Ok(Some(total))
    }

    /// Diagnostics: reads a register block back from the controller.
    pub fn read_back(&mut self, register: u8) -> Result<Vec<u8>> {
        self.send(register, 0, 0, 0)?;
        self.query()
    }

    fn set_builtin_effects(&mut self, zone: &Fusion2Zone, enabled: bool) -> Result<()> {
        let base = if self.builtin_disabled_mask < 0 {
            0
        } else {
            self.builtin_disabled_mask
        };
        let mask = if enabled {
            base & !(zone.builtin_effect_bit as i32)
        } else {
            base | zone.builtin_effect_bit as i32
        };
        if self.builtin_disabled_mask >= 0 && mask == self.builtin_disabled_mask {
            return Ok(());
        }
        self.builtin_disabled_mask = mask;
        self.send(0x32, mask as u8, 0, 0)?;
        thread::sleep(Duration::from_millis(50));
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        for register in 0x20..=0x27u8 {
            self.send(register, 0, 0, 0)?;
        }
        if self.info.product_id == 0x5711 {
            for register in 0x90..=0x92u8 {
                self.send(register, 0, 0, 0)?;
            }
        }
        self.apply()?;
        Ok(())
    }

    fn query(&mut self) -> Result<Vec<u8>> {
        let reply = self.hid.get_feature(REPORT_ID, PACKET_SIZE)?;
        if reply.len() < PACKET_SIZE {
            bail!("short feature reply: {} bytes", reply.len());
        }
        if reply[0] != REPORT_ID {
            bail!("unexpected report id 0x{:02X}", reply[0]);
        }
        Ok(reply)
    }

    fn send(&mut self, a: u8, b: u8, c: u8, d: u8) -> Result<()> {
        self.hid.set_feature(&simple(a, b, c, d))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_firmware_and_capability_fields() {
        let mut reply = [0u8; PACKET_SIZE];
        reply[0] = REPORT_ID;
        reply[2] = 0; // device num
        reply[3] = 1; // strip detect
        reply[4..8].copy_from_slice(&0x0000_2906u32.to_le_bytes()); // 6.41.0.0
        reply[11] = 0x03;
        reply[12..16].copy_from_slice(b"IT57");
        reply[44..48].copy_from_slice(&0x0001_0200u32.to_le_bytes());
        reply[48..52].copy_from_slice(&0x0000_0102u32.to_le_bytes());
        reply[56..60].copy_from_slice(&0x0000_5711u32.to_le_bytes());

        let firmware_raw = le_u32(&reply[4..8]);
        assert_eq!(
            format!(
                "{}.{}.{}.{}",
                firmware_raw & 0xFF,
                (firmware_raw >> 8) & 0xFF,
                (firmware_raw >> 16) & 0xFF,
                (firmware_raw >> 24) & 0xFF
            ),
            "6.41.0.0"
        );
        assert_eq!(le_u32(&reply[56..60]), 0x5711);
        assert_eq!(le_u32(&reply[44..48]), 0x0001_0200);
    }

    #[test]
    fn static_effect_packet_layout() {
        let zone = &B850_ZONES[0];
        let packet = encode_effect(
            zone,
            Fusion2Effect::Static,
            RgbColor::new(0x11, 0x22, 0x33),
            50,
            50,
            false,
        );
        assert_eq!(packet[0], REPORT_ID);
        assert_eq!(packet[1], 0x25);
        assert_eq!(
            u32::from_le_bytes([packet[2], packet[3], packet[4], packet[5]]),
            1 << 5
        );
        assert_eq!(packet[11], Fusion2Effect::Static as u8);
        assert_eq!(packet[12], 127); // 50% of 255, truncated
        assert_eq!(
            u32::from_le_bytes([packet[14], packet[15], packet[16], packet[17]]),
            0x0011_2233
        );
    }

    #[test]
    fn apply_packet_targets_the_it5711() {
        let packet = encode_apply(0x5711);
        assert_eq!(&packet[..4], &[REPORT_ID, 0x28, 0xFF, 0x07]);
    }

    #[test]
    fn direct_mode_splits_at_19_leds_and_uses_grb() {
        let colors = vec![RgbColor::new(1, 2, 3); 20];
        let packets = encode_direct(&B850_ZONES[0], &colors, 0x0001_0002);
        assert_eq!(packets.len(), 2);

        assert_eq!(packets[0][1], 0x58);
        assert_eq!(u16::from_le_bytes([packets[0][2], packets[0][3]]), 0);
        assert_eq!(packets[0][4], 19 * 3);
        // R at slot 1, G at slot 0, B at slot 2 -> GRB.
        assert_eq!(&packets[0][5..8], &[2, 1, 3]);

        assert_eq!(u16::from_le_bytes([packets[1][2], packets[1][3]]), 57);
        assert_eq!(packets[1][4], 3);
    }

    #[test]
    fn direct_mode_rejects_single_led_zones() {
        assert!(encode_direct(&B850_ZONES[3], &[RgbColor::WHITE], 0).is_empty());
    }

    #[test]
    fn reset_ends_with_apply_and_covers_gen2_registers() {
        let packets = encode_reset(0x5711);
        assert_eq!(packets.len(), 8 + 3 + 1);
        assert_eq!(packets[0][1], 0x20);
        assert_eq!(packets[7][1], 0x27);
        assert_eq!(packets[8][1], 0x90);
        assert_eq!(packets.last().unwrap()[1], 0x28);
    }
}
