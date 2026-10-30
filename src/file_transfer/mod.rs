//! Built-in device file transfers. Relay carries control; object storage carries bytes.
mod client;
mod jobs;
pub(crate) use client::{put_source, FileClient, Purpose, UploadSource};
pub(crate) use jobs::FileMethod;
pub(crate) const PROTOCOL_FEATURE: &str = "device_file_transfers_v1";
#[cfg(test)]
mod tests;
