//! Deterministic MyLianPC logic: colours, effects, fan curves and profiles.
//!
//! Nothing in this crate touches hardware or the operating system, so it is
//! fully unit-testable anywhere.

pub mod color;
pub mod device;
pub mod effect;
pub mod fan;
pub mod ipc;
pub mod metrics;
pub mod network;
pub mod profile;
pub mod settings;
