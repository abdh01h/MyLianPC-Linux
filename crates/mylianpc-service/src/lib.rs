//! Background service (systemd): the only process allowed to touch hardware.

pub mod adapters;
pub mod manager;
pub mod paths;
pub mod server;

pub const SERVICE_NAME: &str = "mylianpc";
