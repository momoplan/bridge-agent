//! Built-in device file transfers. Relay carries control; object storage carries bytes.
mod client;
mod jobs;
pub(crate) use client::{put_source, UploadSource};
#[cfg(any(test, target_os = "macos", windows))]
pub(crate) use client::{FileClient, Purpose};
pub(crate) use jobs::FileMethod;
pub(crate) const PROTOCOL_FEATURE: &str = "device_file_transfers_v1";
#[cfg(test)]
mod tests;
