//! Linux SMBus transport through `/dev/i2c-*` (`I2C_SMBUS` ioctl).
//!
//! Write helpers refuse SPD addresses `0x50`-`0x57` unconditionally. Only the
//! Corsair RGB controller registers are in scope.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;

const I2C_SLAVE: libc::c_ulong = 0x0703;
const I2C_SMBUS: libc::c_ulong = 0x0720;
const WRITE: u8 = 0;
const READ: u8 = 1;
const BYTE_DATA: u32 = 2;
const WORD_DATA: u32 = 3;
const BLOCK_DATA: u32 = 5;

#[repr(C)]
struct SmbusIoctlData {
    read_write: u8,
    command: u8,
    size: u32,
    data: *mut u8,
}

pub struct Smbus {
    file: File,
    number: u32,
}

impl Smbus {
    pub fn open(number: u32) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!("/dev/i2c-{number}"))?;
        Ok(Self { file, number })
    }

    pub fn number(&self) -> u32 {
        self.number
    }

    pub fn read_byte_data(&self, address: u8, command: u8) -> io::Result<u8> {
        let mut data = [0u8; 34];
        self.ioctl(address, READ, command, BYTE_DATA, &mut data)?;
        Ok(data[0])
    }

    pub fn write_byte_data(&self, address: u8, command: u8, value: u8) -> io::Result<()> {
        ensure_not_spd(address)?;
        let mut data = [0u8; 34];
        data[0] = value;
        self.ioctl(address, WRITE, command, BYTE_DATA, &mut data)?;
        Ok(())
    }

    pub fn read_word_data(&self, address: u8, command: u8) -> io::Result<u16> {
        let mut data = [0u8; 34];
        self.ioctl(address, READ, command, WORD_DATA, &mut data)?;
        Ok(u16::from_le_bytes([data[0], data[1]]))
    }

    pub fn write_word_data(&self, address: u8, command: u8, value: u16) -> io::Result<()> {
        ensure_not_spd(address)?;
        let mut data = [0u8; 34];
        data[..2].copy_from_slice(&value.to_le_bytes());
        self.ioctl(address, WRITE, command, WORD_DATA, &mut data)?;
        Ok(())
    }

    pub fn read_block_data(&self, address: u8, command: u8) -> io::Result<Vec<u8>> {
        let mut data = [0u8; 34];
        self.ioctl(address, READ, command, BLOCK_DATA, &mut data)?;
        let length = (data[0] as usize).min(32);
        Ok(data[1..1 + length].to_vec())
    }

    pub fn write_block_data(
        &self,
        address: u8,
        command: u8,
        values: &[u8],
    ) -> io::Result<()> {
        ensure_not_spd(address)?;
        if values.len() > 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SMBus block data is at most 32 bytes",
            ));
        }
        let mut data = [0u8; 34];
        data[0] = values.len() as u8;
        data[1..1 + values.len()].copy_from_slice(values);
        self.ioctl(address, WRITE, command, BLOCK_DATA, &mut data)?;
        Ok(())
    }

    fn ioctl(
        &self,
        address: u8,
        read_write: u8,
        command: u8,
        size: u32,
        data: &mut [u8; 34],
    ) -> io::Result<i32> {
        // i2c-dev carries the slave address on the file descriptor, not in the
        // SMBus request. i2cget does this implicitly; without it every
        // transaction fails with ENXIO.
        let selected = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                I2C_SLAVE,
                address as libc::c_ulong,
            )
        };
        if selected < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut request = SmbusIoctlData {
            read_write,
            command,
            size,
            data: data.as_mut_ptr(),
        };
        let result = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                I2C_SMBUS,
                &mut request as *mut SmbusIoctlData as *mut libc::c_void,
            )
        };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(result)
        }
    }
}

fn ensure_not_spd(address: u8) -> io::Result<()> {
    if (0x50..=0x57).contains(&address) {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing to write an SPD address (0x50-0x57)",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_guard_refuses_spd_addresses() {
        for address in 0x50..=0x57 {
            assert!(ensure_not_spd(address).is_err(), "0x{address:02X}");
        }
        assert!(ensure_not_spd(0x19).is_ok());
        assert!(ensure_not_spd(0x58).is_ok());
    }
}
