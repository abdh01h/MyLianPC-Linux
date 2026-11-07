//! Minimal raw USB access through the kernel's `usbfs` interface.
//!
//! The HydroShift II has no kernel driver bound to its vendor interface, so
//! claiming the interface and issuing bulk transfers directly is enough. Using
//! `usbfs` keeps the service dependency-free (no libusb runtime linkage) while
//! behaving like the WinUSB transport used by the Windows implementation.
//!
//! Only bulk endpoints are supported; that is all the cooler uses.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::inventory;

// Linux `usbdevice_fs.h` ioctl numbers (x86_64, 64-bit pointers).
// _IOR('U', 15, unsigned int)
const USBDEVFS_CLAIMINTERFACE: libc::c_ulong = 0x8004_550F;
// _IOR('U', 16, unsigned int)
const USBDEVFS_RELEASEINTERFACE: libc::c_ulong = 0x8004_5510;
// _IOWR('U', 2, struct usbdevfs_bulktransfer)
const USBDEVFS_BULK: libc::c_ulong = 0xC018_5502;

#[repr(C)]
struct BulkTransfer {
    ep: u32,
    len: u32,
    timeout: u32,
    data: *mut libc::c_void,
}

/// One open interface on a USB device.
pub struct UsbInterface {
    file: Arc<File>,
    interface: u32,
    path: PathBuf,
    claimed: bool,
    reader: Option<ReplyReader>,
}

struct ReplyReader {
    stop: Arc<AtomicBool>,
    replies: Arc<(Mutex<ReplyQueue>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct ReplyQueue {
    packets: VecDeque<Vec<u8>>,
    closed: bool,
}

impl UsbInterface {
    /// Opens the raw device node and claims `interface` (usually 0).
    pub fn open(path: impl AsRef<Path>, interface: u32) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new().read(true).write(true).open(&path)?;
        let fd = file.as_raw_fd();
        let number = interface;
        // SAFETY: the ioctl only reads the integer argument.
        let result = unsafe { libc::ioctl(fd, USBDEVFS_CLAIMINTERFACE, &number as *const u32) };
        if result < 0 {
            let error = io::Error::last_os_error();
            return Err(match error.raw_os_error() {
                Some(libc::EBUSY) => io::Error::new(
                    io::ErrorKind::ResourceBusy,
                    format!("USB interface {interface} is already in use: {error}"),
                ),
                Some(libc::EACCES) => io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "no permission to claim {} (add the user to the mylianpc group): {error}",
                        path.display()
                    ),
                ),
                _ => error,
            });
        }
        Ok(Self {
            file: Arc::new(file),
            interface,
            path,
            claimed: true,
            reader: None,
        })
    }

    /// Finds and opens the first USB device with the given VID/PID, claiming the
    /// interface that owns the requested endpoints.
    pub fn open_by_ids(vid: &str, pid: &str) -> io::Result<Self> {
        let device = inventory::usb_devices()
            .into_iter()
            .find(|device| {
                device.vid.eq_ignore_ascii_case(vid) && device.pid.eq_ignore_ascii_case(pid)
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("USB device {vid}:{pid} was not found"),
                )
            })?;
        let node = format!(
            "/dev/bus/usb/{:03}/{:03}",
            device.bus, device.dev
        );
        let interface = device
            .interfaces
            .iter()
            .find(|interface| {
                interface
                    .endpoints
                    .iter()
                    .any(|endpoint| endpoint.address == 0x01)
            })
            .map(|interface| interface.number as u32)
            .unwrap_or(0);
        Self::open(node, interface)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes one bulk transfer to `endpoint`.
    pub fn write_bulk(&self, endpoint: u8, data: &[u8], timeout_ms: u32) -> io::Result<()> {
        let mut buffer = data.to_vec();
        let transferred = self.bulk(endpoint, &mut buffer, timeout_ms, false)?;
        if transferred != data.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!("short USB write: {transferred}/{} bytes", data.len()),
            ));
        }
        Ok(())
    }

    /// Reads one bulk transfer from `endpoint`.
    pub fn read_bulk(
        &self,
        endpoint: u8,
        length: usize,
        timeout_ms: u32,
    ) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0u8; length];
        let transferred = self.bulk(endpoint, &mut buffer, timeout_ms, true)?;
        buffer.truncate(transferred);
        Ok(buffer)
    }

    fn bulk(
        &self,
        endpoint: u8,
        data: &mut [u8],
        timeout_ms: u32,
        read: bool,
    ) -> io::Result<usize> {
        let mut transfer = BulkTransfer {
            ep: endpoint as u32 | if read { 0x80 } else { 0 },
            len: data.len() as u32,
            timeout: timeout_ms,
            data: data.as_mut_ptr().cast(),
        };
        // SAFETY: `transfer` points at a live, correctly sized buffer for the
        // duration of the ioctl.
        let result = unsafe {
            libc::ioctl(self.file.as_raw_fd(), USBDEVFS_BULK, &mut transfer as *mut BulkTransfer)
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ETIMEDOUT) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("USB bulk transfer on 0x{endpoint:02X} timed out: {error}"),
                ));
            }
            return Err(error);
        }
        Ok(result as usize)
    }

    /// Starts the background reader that keeps one IN transfer pending. Replies
    /// are queued so a slow caller never lets the device's transmit buffer fill.
    pub fn start_reply_reader(&mut self, endpoint: u8, max_packet: usize) {
        if self.reader.is_some() {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let replies = Arc::new((Mutex::new(ReplyQueue::default()), Condvar::new()));
        let file = Arc::clone(&self.file);
        let stop_thread = Arc::clone(&stop);
        let queue = Arc::clone(&replies);
        let thread = thread::Builder::new()
            .name(format!("usbfs-reader-{endpoint:02X}"))
            .spawn(move || {
                while !stop_thread.load(Ordering::Acquire) {
                    let mut buffer = vec![0u8; max_packet];
                    let mut transfer = BulkTransfer {
                        ep: endpoint as u32 | 0x80,
                        len: max_packet as u32,
                        timeout: 2000,
                        data: buffer.as_mut_ptr().cast(),
                    };
                    // SAFETY: same contract as `bulk` above.
                    let result = unsafe {
                        libc::ioctl(file.as_raw_fd(), USBDEVFS_BULK, &mut transfer)
                    };
                    if result < 0 {
                        let error = io::Error::last_os_error();
                        if error.raw_os_error() == Some(libc::ETIMEDOUT) {
                            continue;
                        }
                        if stop_thread.load(Ordering::Acquire) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                    if result > 0 {
                        buffer.truncate(result as usize);
                        let (lock, wake) = &*queue;
                        if let Ok(mut queue) = lock.lock() {
                            if queue.packets.len() >= 64 {
                                queue.packets.pop_front();
                            }
                            queue.packets.push_back(buffer);
                            wake.notify_one();
                        }
                    }
                }
                if let Ok(mut queue) = queue.0.lock() {
                    queue.closed = true;
                }
                queue.1.notify_all();
            })
            .ok();
        self.reader = Some(ReplyReader {
            stop,
            replies,
            thread,
        });
    }

    /// Waits for the next queued reply.
    pub fn take_reply(&self, timeout: Duration) -> Option<Vec<u8>> {
        let reader = self.reader.as_ref()?;
        let (lock, wake) = &*reader.replies;
        let mut queue = lock.lock().ok()?;
        if let Some(packet) = queue.packets.pop_front() {
            return Some(packet);
        }
        let (mut queue, _) = wake
            .wait_timeout(queue, timeout)
            .ok()?;
        queue.packets.pop_front()
    }

    /// Discards replies that arrived after their command already timed out.
    pub fn drain_replies(&self) -> usize {
        let Some(reader) = self.reader.as_ref() else {
            return 0;
        };
        let (lock, _) = &*reader.replies;
        let Ok(mut queue) = lock.lock() else {
            return 0;
        };
        let count = queue.packets.len();
        queue.packets.clear();
        count
    }
}

impl Drop for UsbInterface {
    fn drop(&mut self) {
        if let Some(reader) = self.reader.take() {
            reader.stop.store(true, Ordering::Release);
            let (lock, wake) = &*reader.replies;
            if let Ok(mut queue) = lock.lock() {
                queue.closed = true;
                wake.notify_all();
            }
            if let Some(thread) = reader.thread {
                let _ = thread.join();
            }
        }
        if self.claimed {
            let number = self.interface;
            // SAFETY: the ioctl only reads the integer argument.
            unsafe {
                libc::ioctl(
                    self.file.as_raw_fd(),
                    USBDEVFS_RELEASEINTERFACE,
                    &number as *const u32,
                );
            }
            self.claimed = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_transfer_matches_the_kernel_layout() {
        // ep, len, timeout, data pointer (8 bytes on x86_64), 4 bytes padding.
        assert_eq!(std::mem::size_of::<BulkTransfer>(), 24);
        let transfer = BulkTransfer {
            ep: 0x81,
            len: 512,
            timeout: 2000,
            data: std::ptr::null_mut(),
        };
        assert_eq!(transfer.ep, 0x81);
    }

    #[test]
    fn ioctl_numbers_match_linux_usbdevice_fs() {
        assert_eq!(USBDEVFS_CLAIMINTERFACE, 0x8004_550F);
        assert_eq!(USBDEVFS_RELEASEINTERFACE, 0x8004_5510);
        assert_eq!(USBDEVFS_BULK, 0xC018_5502);
    }
}
