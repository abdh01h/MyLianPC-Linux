//! Linux hardware access for MyLianPC.
//!
//! The inventory and the Fusion2 identification query are read-only. There is
//! no effect, colour, fan or flash write wired up yet.

pub mod corsair;
pub mod fusion2;
pub mod hidraw;
pub mod hydroshift2;
pub mod hydroshift_device;
pub mod inventory;
pub mod network;
pub mod nvme;
pub mod nvml;
pub mod rapl;
pub mod sensors;
pub mod smbios;
pub mod smbus;
pub mod tinyuz;
pub mod usbfs;
