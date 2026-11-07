//! Minimal Linux `hidraw` transport.
//!
//! Feature-report helpers are included because the Fusion2 protocol is built
//! on HID feature reports. Only the read/query side is used by the probe; the
//! write path is not wired to any UI or service command yet.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

/// `_IOR('H', 0x01, int)`.
const HIDIOCGRDESCSIZE: libc::c_ulong = 0x8004_4801;
/// `_IOR('H', 0x02, struct hidraw_report_descriptor)`.
const HIDIOCGRDESC: libc::c_ulong = 0x9004_4802;

const MAX_DESCRIPTOR_SIZE: usize = 4096;

#[repr(C)]
struct ReportDescriptor {
    size: u32,
    value: [u8; MAX_DESCRIPTOR_SIZE],
}

pub struct HidRaw {
    file: File,
    path: PathBuf,
}

impl HidRaw {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.as_ref())?;
        Ok(Self {
            file,
            path: path.as_ref().to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the HID report descriptor through `HIDIOCGRDESC`.
    pub fn report_descriptor(&self) -> io::Result<Vec<u8>> {
        let mut size: libc::c_int = 0;
        let result = unsafe { libc::ioctl(self.file.as_raw_fd(), HIDIOCGRDESCSIZE, &mut size) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        if size <= 0 || size as usize > MAX_DESCRIPTOR_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid HID report descriptor size {size}"),
            ));
        }

        let mut descriptor = ReportDescriptor {
            size: size as u32,
            value: [0; MAX_DESCRIPTOR_SIZE],
        };
        let result = unsafe { libc::ioctl(self.file.as_raw_fd(), HIDIOCGRDESC, &mut descriptor) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(descriptor.value[..descriptor.size as usize].to_vec())
    }

    /// Reads a feature report. `len` includes the report ID byte, matching the
    /// 64-byte Fusion2 packet size.
    pub fn get_feature(&self, report_id: u8, len: usize) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0u8; len];
        buffer[0] = report_id;
        let request = feature_ioctl(0x07, len);
        let result = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                request,
                buffer.as_mut_ptr() as *mut libc::c_void,
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(buffer)
    }

    /// Writes a feature report. Used only by explicit protocol operations; the
    /// read-only inventory never calls it.
    pub fn set_feature(&self, data: &[u8]) -> io::Result<()> {
        let request = feature_ioctl(0x06, data.len());
        let result = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                request,
                data.as_ptr() as *mut libc::c_void,
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// `_IOC(_IOC_WRITE|_IOC_READ, 'H', nr, len)`.
fn feature_ioctl(nr: u8, len: usize) -> libc::c_ulong {
    0xC000_0000
        | ((len as libc::c_ulong) << 16)
        | ((b'H' as libc::c_ulong) << 8)
        | nr as libc::c_ulong
}

/// True when the descriptor contains the given usage page/usage pair.
///
/// This is a deliberately small HID item parser: it tracks the global Usage
/// Page and the local Usage long enough to identify a vendor collection.
pub fn has_usage(descriptor: &[u8], usage_page: u16, usage: u16) -> bool {
    let mut index = 0;
    let mut current_page: u16 = 0;

    while index < descriptor.len() {
        let prefix = descriptor[index];
        index += 1;

        if prefix == 0xFE {
            // Long item: data size byte, long-item tag, then data.
            if index + 2 > descriptor.len() {
                return false;
            }
            let data_size = descriptor[index] as usize;
            index += 2 + data_size;
            continue;
        }

        let size = match prefix & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        let item_type = (prefix >> 2) & 0x03;
        let tag = prefix >> 4;

        if index + size > descriptor.len() {
            return false;
        }
        let data = &descriptor[index..index + size];
        index += size;

        match (item_type, tag) {
            // Global: Usage Page.
            (1, 0) => {
                current_page = read_unsigned(data) as u16;
            }
            // Local: Usage. A 4-byte usage carries page and id; the low 16 bits
            // are the usage id.
            (2, 0) => {
                let value = read_unsigned(data);
                let (page, id) = if size == 4 {
                    (((value >> 16) & 0xFFFF) as u16, (value & 0xFFFF) as u16)
                } else {
                    (current_page, value as u16)
                };
                if page == usage_page && id == usage {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn read_unsigned(data: &[u8]) -> u32 {
    let mut value = 0u32;
    for (shift, byte) in data.iter().enumerate() {
        value |= (*byte as u32) << (shift * 8);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_linux_hidraw_h() {
        // HIDIOCGFEATURE(64) = _IOC(_IOC_WRITE|_IOC_READ, 'H', 0x07, 64)
        assert_eq!(feature_ioctl(0x07, 64), 0xC040_4807);
        assert_eq!(feature_ioctl(0x06, 64), 0xC040_4806);
    }

    #[test]
    fn finds_the_vendor_usage_in_the_real_descriptor_prefix() {
        // Prefix captured from /dev/hidraw8 on the B850 board.
        let descriptor = [
            0x06, 0x89, 0xFF, 0x09, 0x10, 0xA1, 0x01, 0x85, 0x5A, 0x09, 0x01, 0x15, 0x00, 0x26,
            0xFF, 0x00, 0x75, 0x08, 0x95, 0x10, 0xB1, 0x00, 0xC0, 0x06, 0x89, 0xFF, 0x09, 0xCC,
            0xA1, 0x01, 0x85, 0xCC,
        ];
        assert!(has_usage(&descriptor, 0xFF89, 0x00CC));
        assert!(has_usage(&descriptor, 0xFF89, 0x0010));
        assert!(!has_usage(&descriptor, 0x0001, 0x00CC));
    }

    #[test]
    fn rejects_a_random_descriptor() {
        assert!(!has_usage(&[0x05, 0x01, 0x09, 0x06], 0xFF89, 0x00CC));
    }
}
