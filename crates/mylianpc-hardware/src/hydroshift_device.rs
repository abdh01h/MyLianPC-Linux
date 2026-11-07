//! Lian Li HydroShift II transport.
//!
//! Ported from `MyLianPC.Hardware/LianLi/HydroShift2Device.cs`. The packet
//! format lives in [`crate::hydroshift2`]; this module owns the USB channel and
//! implements the behaviours learned on the real cooler:
//!
//! * every command gets a 512-byte reply, and replies that are never read fill
//!   the controller's transmit buffer until it stops accepting writes, so a
//!   background reader keeps one IN transfer pending at all times;
//! * L-Connect leaves the panel playing H.264 with a PNG overlay, so
//!   [`HydroShiftDevice::initialize`] stops playback and clears both layers;
//! * pump/fan writes are limited to the firmware-safe 3.6 s cadence, and the
//!   setpoint is repeated every 5 s by the caller because the firmware falls
//!   back to its own curve after about ten seconds.

use std::io;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::Utc;

use crate::hydroshift2::{self as proto, Telemetry};
use crate::usbfs::UsbInterface;

const REPLY_TIMEOUT: Duration = Duration::from_secs(2);
const WRITE_TIMEOUT_MS: u32 = 5000;
const MIN_PUMP_FAN_INTERVAL: Duration = Duration::from_millis(3600);
const MAX_JPEG_BYTES: usize = 153_600;
const MAX_SILENT_REPLIES: u32 = 3;

#[derive(Debug)]
pub enum HydroShiftError {
    Io(io::Error),
    Unresponsive,
    InvalidPacket(String),
    DeviceRejected(String),
}

impl std::fmt::Display for HydroShiftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Unresponsive => write!(
                f,
                "the HydroShift II controller stopped answering. If it does not recover on its own, \
                 shut the PC down and switch the PSU off for 15 seconds."
            ),
            Self::InvalidPacket(message) => write!(f, "{message}"),
            Self::DeviceRejected(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for HydroShiftError {}

impl From<io::Error> for HydroShiftError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, HydroShiftError>;

/// One open HydroShift II controller.
pub struct HydroShiftDevice {
    usb: UsbInterface,
    firmware: Option<String>,
    last_reply: Option<Instant>,
    silent_replies: u32,
    last_pump_fan: Option<Instant>,
}

impl HydroShiftDevice {
    /// Opens the cooler by USB ID and starts its reply reader.
    pub fn open() -> Result<Self> {
        let mut usb = UsbInterface::open_by_ids("1cbe", "a021")?;
        usb.start_reply_reader(proto::IN_ENDPOINT, 512);
        Ok(Self {
            usb,
            firmware: None,
            last_reply: None,
            silent_replies: 0,
            last_pump_fan: None,
        })
    }

    pub fn firmware(&self) -> Option<&str> {
        self.firmware.as_deref()
    }

    pub fn last_reply(&self) -> Option<Instant> {
        self.last_reply
    }

    pub fn unresponsive(&self) -> bool {
        self.silent_replies >= MAX_SILENT_REPLIES
    }

    /// Sends one packet and waits for its reply. `Ok(None)` means no reply
    /// arrived within two seconds (the caller decides whether that is fatal).
    fn exchange(&mut self, packet: &[u8]) -> Result<Option<Vec<u8>>> {
        if self.unresponsive() {
            return Err(HydroShiftError::Unresponsive);
        }
        self.exchange_core(packet)
    }

    fn exchange_core(&mut self, packet: &[u8]) -> Result<Option<Vec<u8>>> {
        let _ = self.usb.drain_replies();
        match self
            .usb
            .write_bulk(proto::OUT_ENDPOINT, packet, WRITE_TIMEOUT_MS)
        {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                // A wedged controller NAKs every OUT transfer; only removing power clears it.
                self.silent_replies = MAX_SILENT_REPLIES;
                return Err(HydroShiftError::Io(error));
            }
            Err(error) => return Err(HydroShiftError::Io(error)),
        }
        match self.usb.take_reply(REPLY_TIMEOUT) {
            Some(reply) => {
                self.silent_replies = 0;
                self.last_reply = Some(Instant::now());
                Ok(Some(reply))
            }
            None => {
                self.silent_replies += 1;
                Ok(None)
            }
        }
    }

    /// Re-arms the control channel after playback (StopPlay, StopClock,
    /// GetVersion) exactly as the vendor driver does.
    pub fn try_wake(&mut self) -> bool {
        self.silent_replies = 0;
        let mut last = None;
        let packets = [
            proto::command(proto::cmd::STOP_PLAY, &[]),
            proto::command(proto::cmd::STOP_CLOCK, &[0]),
            proto::command(proto::cmd::GET_VERSION, &[]),
        ];
        for packet in packets {
            if let Ok(Some(reply)) = self.exchange_core(&packet) {
                last = Some(reply);
            }
            if self.unresponsive() {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        if let Some(reply) = last {
            if let Some(version) = proto::parse_version(&reply) {
                self.firmware = Some(version);
            }
        }
        let alive = self
            .last_reply
            .map(|at| at.elapsed() < Duration::from_secs(5))
            .unwrap_or(false);
        if !alive {
            self.silent_replies = MAX_SILENT_REPLIES;
        }
        alive
    }

    /// Wake, sync the clock, stop the built-in clock widget, clear both layers
    /// and set 30 fps. This is a hardware write and is only called when the
    /// service takes ownership of the cooler.
    pub fn initialize(&mut self) -> Result<()> {
        if !self.try_wake() {
            return Err(HydroShiftError::Unresponsive);
        }
        let packets: Vec<Vec<u8>> = vec![
            proto::clock(Utc::now()).to_vec(),
            proto::command(proto::cmd::STOP_CLOCK, &[0]).to_vec(),
            proto::image(transparent_png(), true, None),
            proto::image(black_jpeg(), false, None),
            proto::command(proto::cmd::FRAME_RATE, &[30]).to_vec(),
        ];
        for packet in packets {
            self.exchange_core(&packet)?;
        }
        if self.unresponsive() {
            return Err(HydroShiftError::Unresponsive);
        }
        Ok(())
    }

    /// Shows a baseline JPEG as the background layer.
    pub fn show_jpeg(&mut self, jpeg: &[u8]) -> Result<bool> {
        if jpeg.len() > MAX_JPEG_BYTES {
            return Err(HydroShiftError::DeviceRejected(format!(
                "JPEG is {} bytes; the panel accepts at most {MAX_JPEG_BYTES}.",
                jpeg.len()
            )));
        }
        Ok(self
            .exchange(&proto::image(jpeg, false, None))?
            .is_some())
    }

    /// Shows a PNG as the transparent overlay layer.
    pub fn show_png_overlay(&mut self, png: &[u8]) -> Result<bool> {
        Ok(self
            .exchange(&proto::image(png, true, None))?
            .is_some())
    }

    pub fn set_brightness(&mut self, percent: i32) -> Result<bool> {
        let value = percent.clamp(0, 100) as u8;
        Ok(self
            .exchange(&proto::command(proto::cmd::BRIGHTNESS, &[value]))?
            .is_some())
    }

    /// Rotation in 90° steps (0-3).
    pub fn set_rotation(&mut self, quarter_turns: i32) -> Result<bool> {
        let value = (quarter_turns & 3) as u8;
        Ok(self
            .exchange(&proto::command(proto::cmd::ROTATION, &[value]))?
            .is_some())
    }

    /// Coolant temperature, pump and fan RPM, or `None` if no plausible reading
    /// was obtained.
    pub fn read_telemetry(&mut self) -> Result<Option<Telemetry>> {
        let Some(reply) = self.exchange(&proto::command(proto::cmd::GET_PARAMS, &[]))? else {
            return Ok(None);
        };
        Ok(proto::parse_telemetry(&reply).filter(|telemetry| {
            (5..80).contains(&telemetry.coolant_c) && telemetry.pump_rpm < 5000
        }))
    }

    /// Sets pump RPM and the three fan duties (percent). Duties are floored at
    /// `min_fan_percent`; the pump is floored at 1600 RPM. Writes are limited to
    /// the 3.6 s cadence unless something is being raised to full speed.
    ///
    /// Returns `false` if the write was deferred by the rate limiter.
    pub fn set_pump_and_fans(
        &mut self,
        pump_rpm: i32,
        fan1_percent: i32,
        fan2_percent: i32,
        fan3_percent: i32,
        min_fan_percent: i32,
    ) -> Result<bool> {
        let raw = |percent: i32, minimum: i32| -> u8 {
            ((percent.clamp(minimum, 100) as f64) * 255.0 / 100.0).round() as u8
        };
        let urgent = pump_rpm >= proto::PUMP_MAX_RPM_CIRCLE as i32
            || fan1_percent >= 100
            || fan2_percent >= 100
            || fan3_percent >= 100;
        if let Some(last) = self.last_pump_fan {
            if last.elapsed() < MIN_PUMP_FAN_INTERVAL && !urgent {
                return Ok(false);
            }
        }
        let parameters = proto::sync_pump_fan_parameters(
            pump_rpm.max(proto::PUMP_MIN_RPM as i32),
            raw(fan1_percent, min_fan_percent),
            raw(fan2_percent, min_fan_percent),
            raw(fan3_percent, min_fan_percent),
        );
        self.exchange(&proto::command(proto::cmd::SYNC_PUMP_FAN, &parameters))?;
        self.last_pump_fan = Some(Instant::now());
        Ok(true)
    }

    /// Uploads a ring animation. The device loops it by itself. If the
    /// firmware does not acknowledge, the channel is re-armed and the upload is
    /// tried once more.
    pub fn set_ring(&mut self, frames: &[Vec<u8>], interval_ticks: u8) -> Result<bool> {
        self.set_ring_with_retry(frames, interval_ticks, true)
    }

    /// Sends one ring upload. The combined ring+fans packet must use
    /// `retry = false`: a rejected fan upload can leave the firmware needing a
    /// power cycle, so it must never be retried automatically.
    pub fn set_ring_with_retry(
        &mut self,
        frames: &[Vec<u8>],
        interval_ticks: u8,
        retry: bool,
    ) -> Result<bool> {
        let packet = proto::ring_frames(frames, interval_ticks, None)
            .map_err(HydroShiftError::InvalidPacket)?;
        let attempts = if retry { 2 } else { 1 };
        for attempt in 0..attempts {
            let reply = self.exchange(&packet)?;
            if let Some(reply) = reply {
                if reply.first() == Some(&proto::cmd::PUSH_RGB) {
                    return Ok(true);
                }
            }
            if attempt + 1 < attempts && !self.try_wake() {
                return Ok(false);
            }
        }
        Ok(false)
    }

    /// Fixed, safe cooling used when the service stops.
    pub fn safe_defaults(&mut self) -> Result<()> {
        self.last_pump_fan = None;
        self.set_pump_and_fans(2500, 70, 70, 70, 20)?;
        Ok(())
    }

    /// Read-only query used by the probe: reads the firmware version and the
    /// telemetry block without waking, initialising or changing anything.
    pub fn query_only(&mut self) -> Result<(Option<String>, Option<Telemetry>)> {
        let version = self
            .exchange(&proto::command(proto::cmd::GET_VERSION, &[]))?
            .as_deref()
            .and_then(proto::parse_version);
        let telemetry = self
            .exchange(&proto::command(proto::cmd::GET_PARAMS, &[]))?
            .as_deref()
            .and_then(proto::parse_telemetry);
        Ok((version, telemetry))
    }

    /// Diagnostics: sends one packet and reports whether a reply arrived.
    pub fn probe(&mut self, packet: &[u8], wait: Duration) -> Result<(Option<Vec<u8>>, Duration)> {
        let _ = self.usb.drain_replies();
        let started = Instant::now();
        self.usb
            .write_bulk(proto::OUT_ENDPOINT, packet, WRITE_TIMEOUT_MS)?;
        let reply = self.usb.take_reply(wait);
        Ok((reply, started.elapsed()))
    }
}

fn transparent_png() -> &'static [u8] {
    static IMAGE: OnceLock<Vec<u8>> = OnceLock::new();
    IMAGE.get_or_init(|| {
        let image = image::RgbaImage::from_pixel(
            proto::WIDTH,
            proto::HEIGHT,
            image::Rgba([0, 0, 0, 0]),
        );
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("encoding the transparent PNG");
        bytes
    })
}

fn black_jpeg() -> &'static [u8] {
    static IMAGE: OnceLock<Vec<u8>> = OnceLock::new();
    IMAGE.get_or_init(|| {
        let image = image::RgbImage::from_pixel(proto::WIDTH, proto::HEIGHT, image::Rgb([0, 0, 0]));
        let mut bytes = Vec::new();
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90);
        encoder
            .encode_image(&image::DynamicImage::ImageRgb8(image))
            .expect("encoding the black JPEG");
        bytes
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_placeholder_images_have_the_expected_format() {
        let png = transparent_png();
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        let jpeg = black_jpeg();
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        assert!(jpeg.len() < MAX_JPEG_BYTES);
    }

    #[test]
    fn the_silent_reply_limit_matches_the_transport() {
        assert_eq!(MAX_SILENT_REPLIES, 3);
        assert_eq!(REPLY_TIMEOUT, Duration::from_secs(2));
        assert_eq!(MIN_PUMP_FAN_INTERVAL, Duration::from_millis(3600));
    }
}
