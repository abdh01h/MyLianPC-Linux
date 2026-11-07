//! Corsair DRAM RGB detection over SMBus.
//!
//! Ported from `MyLianPC.Hardware/Corsair/CorsairDramController.cs`, which
//! derives from OpenRGB's GPL-2.0-or-later implementation. **Detection and
//! device-info reads only**: the recovery write (`0x23`), effect uploads and
//! colour writes are deliberately not implemented here.

use std::thread;
use std::time::Duration;

use mylianpc_core::color::RgbColor;
use serde::{Deserialize, Serialize};

use crate::inventory;
use crate::smbus::Smbus;

const REG_RESET_BUFFER: u8 = 0x0B;
const REG_SET_BINARY_DATA: u8 = 0x20;
const REG_BINARY_START: u8 = 0x21;
const REG_SWITCH_MODE: u8 = 0x23;
const REG_STATUS: u8 = 0x30;
const REG_GET_BINARY_DATA: u8 = 0x40;
const REG_GET_CHECKSUM: u8 = 0x42;
const REG_GET_DEVICE_INFO: u8 = 0x61;
const REG_SELECT_BUFFER: u8 = 0x63;
const REG_WRITE_CONFIGURATION: u8 = 0x82;
const BUFFER_EFFECT: u8 = 1;
const BUFFER_COLORS: u8 = 2;
const EXPECTED_PROTOCOL_VERSION: u8 = 4;

/// LEDs used for every module. Vengeance RGB DDR5 sticks have 10 or 12; the
/// device info does not say which, and a 10-LED module ignores the surplus.
pub const LED_COUNT: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CorsairMode {
    ColorShift = 0x00,
    ColorPulse = 0x01,
    RainbowWave = 0x03,
    ColorWave = 0x04,
    Visor = 0x05,
    Rain = 0x06,
    Marquee = 0x07,
    Rainbow = 0x08,
    Sequential = 0x09,
    Static = 0x10,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CorsairDram {
    pub address: u8,
    pub vid: u16,
    pub pid: u16,
    pub firmware: String,
    pub protocol_version: u8,
    pub device_info_valid: bool,
    /// True when the module reports an older protocol and would need the
    /// recovery write (`0x23 = 1`). The read-only probe never performs that write.
    pub stalled: bool,
    /// True when this module was just recovered from the abnormal firmware mode.
    #[serde(default)]
    pub recovered: bool,
    /// Outcome of the last static colour write: `true` = read back identical,
    /// `false` = acknowledged but not stored, `None` = could not read back.
    #[serde(default)]
    pub last_write_stored: Option<bool>,
    /// Runtime state: the module is already in the static ("custom") effect.
    #[serde(skip)]
    pub static_mode_set: bool,
}

/// Result of a periodic health check of one module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorsairHealth {
    /// The module answers and reports the normal-mode protocol.
    Normal,
    /// The module was stuck in the abnormal mode and has just been switched
    /// back; its stored lighting has to be rewritten.
    Recovered,
    /// The module answers with verified device info but still reports the
    /// abnormal mode and ignores lighting writes.
    Stalled,
    /// The device-info block could not be read; nothing may be written.
    Unreadable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BusModules {
    pub bus: u32,
    pub adapter: String,
    pub modules: Vec<CorsairDram>,
}

/// Scans every SMBus PIIX4 adapter. Read-only: it never writes an SPD address
/// and never performs the recovery write.
pub fn detect_all() -> Vec<BusModules> {
    let mut found = Vec::new();
    for adapter in inventory::i2c_adapters()
        .into_iter()
        .filter(|adapter| adapter.name.contains("SMBus PIIX4"))
    {
        let Ok(bus) = Smbus::open(adapter.number) else {
            continue;
        };
        let modules = detect_on(&bus);
        if !modules.is_empty() {
            found.push(BusModules {
                bus: adapter.number,
                adapter: adapter.name,
                modules,
            });
        }
    }
    found
}

/// Scans the RGB controller ranges (`0x58`-`0x5F` and `0x18`-`0x1F`) using the
/// `0x43`/`0x44` identification registers.
pub fn detect_on(bus: &Smbus) -> Vec<CorsairDram> {
    let mut found = Vec::new();
    for module in scan(bus) {
        found.push(module);
    }
    found
}

/// Scans and puts a module stuck in the abnormal firmware mode back into normal
/// mode, like the Windows service does while detecting. **This writes
/// `0x23 = 1` to a stalled module and discards the lighting stored on it**, so
/// only the service may call it.
pub fn detect_and_recover_on(bus: &Smbus) -> Vec<CorsairDram> {
    let mut found = scan(bus);
    for module in &mut found {
        if module.device_info_valid
            && module.protocol_version < EXPECTED_PROTOCOL_VERSION
            && module.recover(bus).unwrap_or(false)
        {
            module.recovered = true;
            module.static_mode_set = false;
            module.stalled = false;
            if let Some(info) = read_info(bus, module.address) {
                module.protocol_version = info.protocol_version;
                module.firmware = info.firmware;
            }
        }
    }
    found
}

fn scan(bus: &Smbus) -> Vec<CorsairDram> {
    let mut found = Vec::new();
    for address in (0x58u8..=0x5Fu8).chain(0x18u8..=0x1Fu8) {
        if let Ok(register_43) = bus.read_byte_data(address, 0x43) {
            if matches!(register_43, 0x1A..=0x1C) {
                if let Ok(register_44) = bus.read_byte_data(address, 0x44) {
                    if matches!(register_44, 0x01 | 0x03 | 0x04) {
                        if let Some(module) = read_info(bus, address) {
                            found.push(module);
                        }
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    found
}

impl CorsairDram {
    /// Puts a module stuck in the abnormal firmware mode back into normal
    /// mode. **This writes `0x23 = 1` and discards the lighting stored on the
    /// module**, so it must only run with a verified device-info block.
    pub fn recover(&self, bus: &Smbus) -> std::io::Result<bool> {
        if !self.device_info_valid {
            return Ok(false);
        }
        bus.write_byte_data(self.address, REG_SWITCH_MODE, 1)?;
        thread::sleep(Duration::from_millis(60));
        Ok(read_info(bus, self.address)
            .map(|module| module.protocol_version >= EXPECTED_PROTOCOL_VERSION)
            .unwrap_or(false))
    }

    /// Re-reads the verified device-info block and updates this record.
    pub fn refresh(&mut self, bus: &Smbus) -> bool {
        match read_info(bus, self.address) {
            Some(info) => {
                self.vid = info.vid;
                self.pid = info.pid;
                self.firmware = info.firmware;
                self.protocol_version = info.protocol_version;
                self.device_info_valid = info.device_info_valid;
                self.stalled = info.stalled;
                true
            }
            None => {
                self.device_info_valid = false;
                false
            }
        }
    }

    /// Re-reads the module and recovers it when it has fallen back into the
    /// abnormal firmware mode. An unreadable block is reported as such and
    /// never causes a write.
    pub fn check_health(&mut self, bus: &Smbus) -> CorsairHealth {
        self.recovered = false;
        if !self.refresh(bus) {
            return CorsairHealth::Unreadable;
        }
        if self.protocol_version >= EXPECTED_PROTOCOL_VERSION {
            return CorsairHealth::Normal;
        }
        if !self.recover(bus).unwrap_or(false) {
            return if self.device_info_valid {
                CorsairHealth::Stalled
            } else {
                CorsairHealth::Unreadable
            };
        }
        self.recovered = true;
        self.static_mode_set = false;
        self.refresh(bus);
        CorsairHealth::Recovered
    }

    /// Stores a hardware effect on the module. Speed 0-2 (slow-fast),
    /// brightness 0-255. Returns false when the module rejected the checksum.
    #[allow(clippy::too_many_arguments)]
    pub fn set_effect(
        &mut self,
        bus: &Smbus,
        mode: CorsairMode,
        speed: u8,
        direction: u8,
        random_colors: bool,
        brightness: u8,
        color1: RgbColor,
        color2: RgbColor,
    ) -> std::io::Result<bool> {
        let data = build_effect_data(
            mode,
            speed,
            direction,
            random_colors,
            brightness,
            color1,
            color2,
        );
        self.static_mode_set = false;
        self.upload_configuration(bus, &data, BUFFER_EFFECT)
    }

    /// Static ("Custom") colour: sets the static effect, then uploads the
    /// per-LED colour buffer. This is a real lighting write.
    ///
    /// Like the Windows driver, the effect is only re-written when the mode
    /// changed, the module gets 15 ms before the colours follow, and the stored
    /// buffer is read back. A module that acknowledges but keeps its old
    /// colours is retried once from the effect.
    pub fn set_static_colors(
        &mut self,
        bus: &Smbus,
        colors: &[RgbColor],
    ) -> std::io::Result<bool> {
        self.last_write_stored = None;
        for attempt in 0..2 {
            if !self.static_mode_set {
                if !self.set_effect(
                    bus,
                    CorsairMode::Static,
                    0,
                    2,
                    false,
                    0,
                    RgbColor::BLACK,
                    RgbColor::BLACK,
                )? {
                    return Ok(false);
                }
                self.static_mode_set = true;
                thread::sleep(Duration::from_millis(15));
            }
            let data = build_color_data(colors, LED_COUNT);
            if !self.upload_configuration(bus, &data, BUFFER_COLORS)? {
                return Ok(false);
            }
            thread::sleep(Duration::from_millis(5));
            let stored = self.static_colors_stored(bus, colors);
            self.last_write_stored = stored.as_ref().ok().copied();
            match stored {
                Ok(true) => return Ok(true),
                Ok(false) => {
                    // Acknowledged but not stored: start over from the effect once.
                    self.static_mode_set = false;
                    if attempt == 1 {
                        return Ok(false);
                    }
                }
                // A read-back that fails says nothing about what the module
                // stored; the Windows driver treats that as success too.
                Err(_) => return Ok(true),
            }
        }
        Ok(false)
    }

    /// Reads back a stored configuration buffer (1 = effect, 2 = colours).
    pub fn read_configuration(
        &self,
        bus: &Smbus,
        buffer_id: u8,
        length: usize,
    ) -> std::io::Result<(Vec<u8>, u8)> {
        bus.write_byte_data(self.address, REG_SELECT_BUFFER, buffer_id)?;
        bus.write_byte_data(self.address, REG_BINARY_START, 0)?;
        let mut data = Vec::with_capacity(length);
        for _ in 0..length {
            data.push(bus.read_byte_data(self.address, REG_GET_BINARY_DATA)?);
        }
        let checksum = bus.read_byte_data(self.address, REG_GET_CHECKSUM)?;
        Ok((data, checksum))
    }

    /// True when the stored colour buffer matches `colors` for every LED.
    /// Read-only; a failed read-back is an error, not a mismatch.
    pub fn static_colors_stored(
        &self,
        bus: &Smbus,
        colors: &[RgbColor],
    ) -> std::io::Result<bool> {
        let length = LED_COUNT * 4;
        // The Windows driver compares the buffer only; the checksum register is
        // read to complete the transfer but is not used to decide the result.
        let (data, _checksum) = self.read_configuration(bus, BUFFER_COLORS, length)?;
        Ok(colors_match_count(&data, colors, LED_COUNT))
    }

    fn upload_configuration(
        &self,
        bus: &Smbus,
        data: &[u8],
        buffer_id: u8,
    ) -> std::io::Result<bool> {
        bus.write_byte_data(self.address, REG_RESET_BUFFER, 0)?;
        bus.write_byte_data(self.address, REG_BINARY_START, 0)?;
        for byte in data {
            bus.write_byte_data(self.address, REG_SET_BINARY_DATA, *byte)?;
        }
        if bus.read_byte_data(self.address, REG_GET_CHECKSUM)? != crc8(data) {
            return Ok(false);
        }
        bus.write_byte_data(self.address, REG_WRITE_CONFIGURATION, buffer_id)?;
        // Give the module time to raise its busy flag before polling it.
        thread::sleep(Duration::from_millis(5));
        self.wait_ready(bus)
    }

    fn wait_ready(&self, bus: &Smbus) -> std::io::Result<bool> {
        for _ in 0..20 {
            // A busy module NAKs the status read; that is expected, not fatal.
            if let Ok(status) = bus.read_byte_data(self.address, REG_STATUS) {
                if status & 0x08 == 0 {
                    return Ok(true);
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(false)
    }
}

/// The 20-byte effect-configuration block.
pub fn build_effect_data(
    mode: CorsairMode,
    speed: u8,
    direction: u8,
    random_colors: bool,
    brightness: u8,
    color1: RgbColor,
    color2: RgbColor,
) -> [u8; 20] {
    let mut data = [0u8; 20];
    data[0] = mode as u8;
    data[1] = speed.min(2);
    data[2] = if random_colors { 0 } else { 1 };
    data[3] = direction;
    data[4..7].copy_from_slice(&[color1.r, color1.g, color1.b]);
    data[7] = brightness;
    data[8..11].copy_from_slice(&[color2.r, color2.g, color2.b]);
    data[11] = brightness;
    data
}

/// `R, G, B, 0xFF` per LED; additional LEDs repeat the last colour.
pub fn build_color_data(colors: &[RgbColor], led_count: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(led_count * 4);
    for index in 0..led_count {
        let color = colors
            .get(index)
            .copied()
            .or_else(|| colors.last().copied())
            .unwrap_or(RgbColor::BLACK);
        data.extend_from_slice(&[color.r, color.g, color.b, 0xFF]);
    }
    data
}

/// Compares a stored `R,G,B,0xFF` buffer with the requested colours.
pub fn colors_match(stored: &[u8], colors: &[RgbColor]) -> bool {
    colors_match_count(stored, colors, colors.len())
}

/// Compares every one of `led_count` stored LEDs; a missing colour repeats the
/// last one, exactly like [`build_color_data`].
pub fn colors_match_count(stored: &[u8], colors: &[RgbColor], led_count: usize) -> bool {
    if colors.is_empty() || led_count == 0 {
        return false;
    }
    for index in 0..led_count {
        let color = colors[index.min(colors.len() - 1)];
        let offset = index * 4;
        if stored.len() < offset + 3
            || stored[offset] != color.r
            || stored[offset + 1] != color.g
            || stored[offset + 2] != color.b
        {
            return false;
        }
    }
    true
}

fn read_info(bus: &Smbus, address: u8) -> Option<CorsairDram> {
    for _ in 0..3 {
        if let Some((info, checksum)) = read_device_info_block(bus, address) {
            if is_verifiable(&info, checksum) {
                return Some(parse_info(address, &info));
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    None
}

fn read_device_info_block(bus: &Smbus, address: u8) -> Option<([u8; 32], u8)> {
    bus.write_byte_data(address, REG_GET_DEVICE_INFO, 0).ok()?;
    bus.write_byte_data(address, REG_BINARY_START, 0).ok()?;

    let mut info = [0u8; 32];
    for byte in &mut info {
        *byte = bus.read_byte_data(address, REG_GET_BINARY_DATA).ok()?;
    }
    let checksum = bus.read_byte_data(address, REG_GET_CHECKSUM).ok()?;
    Some((info, checksum))
}

fn parse_info(address: u8, info: &[u8; 32]) -> CorsairDram {
    let vid = (info[1] as u16) << 8 | info[0] as u16;
    let pid = (info[3] as u16) << 8 | info[2] as u16;
    let firmware = format!(
        "{}.{}.{}",
        info[9],
        info[8],
        ((info[11] as u16) << 8) | info[10] as u16
    );
    let protocol_version = info[28];
    CorsairDram {
        address,
        vid,
        pid,
        firmware,
        protocol_version,
        device_info_valid: true,
        stalled: protocol_version < EXPECTED_PROTOCOL_VERSION,
        recovered: false,
        last_write_stored: None,
        static_mode_set: false,
    }
}

/// True when a device-info block can be trusted: matching CRC-8 and not the
/// all-zero block a failed transfer returns (its CRC is also 0).
pub fn is_verifiable(info: &[u8; 32], checksum: u8) -> bool {
    crc8(info) == checksum && (info[0] | info[1] | info[2] | info[3]) != 0
}

/// CRC-8/SMBUS (poly 0x07, init 0).
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for byte in data {
        crc ^= *byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_matches_the_smbus_check_value() {
        assert_eq!(crc8(b"123456789"), 0xF4);
        assert_eq!(crc8(&[]), 0x00);
    }

    #[test]
    fn rejects_the_all_zero_block_even_though_its_crc_matches() {
        let zeros = [0u8; 32];
        assert_eq!(crc8(&zeros), 0);
        assert!(!is_verifiable(&zeros, 0));
    }

    #[test]
    fn parses_device_info_fields() {
        let mut info = [0u8; 32];
        info[0] = 0x1C; // vid 0x1B1C
        info[1] = 0x1B;
        info[2] = 0x01; // pid 0x0901
        info[3] = 0x09;
        info[8] = 1;
        info[9] = 1;
        info[10] = 0x09;
        info[11] = 0;
        info[28] = 4;
        let checksum = crc8(&info);
        assert!(is_verifiable(&info, checksum));

        let module = parse_info(0x19, &info);
        assert_eq!(module.vid, 0x1B1C);
        assert_eq!(module.pid, 0x0901);
        assert_eq!(module.firmware, "1.1.9");
        assert_eq!(module.protocol_version, 4);
        assert!(!module.stalled);
    }

    #[test]
    fn flags_an_older_protocol_as_stalled() {
        let mut info = [0u8; 32];
        info[0] = 1; // make the block non-zero
        info[28] = 1;
        let module = parse_info(0x1B, &info);
        assert!(module.stalled);
    }

    #[test]
    fn effect_block_matches_the_reference_layout() {
        let data = build_effect_data(
            CorsairMode::Static,
            0,
            2,
            false,
            0,
            RgbColor::new(1, 2, 3),
            RgbColor::new(4, 5, 6),
        );
        assert_eq!(data[0], 0x10);
        assert_eq!(data[1], 0);
        assert_eq!(data[2], 1);
        assert_eq!(data[3], 2);
        assert_eq!(&data[4..7], &[1, 2, 3]);
        assert_eq!(&data[8..11], &[4, 5, 6]);
        assert_eq!(&data[12..], &[0u8; 8]);
    }

    #[test]
    fn color_buffer_is_rgba_and_repeats_the_last_color() {
        let data = build_color_data(
            &[RgbColor::new(10, 20, 30), RgbColor::new(40, 50, 60)],
            3,
        );
        assert_eq!(data.len(), 12);
        assert_eq!(&data[0..4], &[10, 20, 30, 0xFF]);
        assert_eq!(&data[4..8], &[40, 50, 60, 0xFF]);
        assert_eq!(&data[8..12], &[40, 50, 60, 0xFF]);
    }

    #[test]
    fn color_comparison_checks_rgb_only() {
        let colors = [RgbColor::new(1, 2, 3)];
        assert!(colors_match(&[1, 2, 3, 0x00], &colors));
        assert!(!colors_match(&[1, 2, 4, 0xFF], &colors));
        assert!(!colors_match(&[1, 2, 3, 0xFF], &[]));
    }
}
