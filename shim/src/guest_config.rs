//! Guest configuration types shared between host and guest.
//!
//! This module defines types that are serialized/deserialized between
//! the host (macOS shim) and guest (Linux init process).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeMount {
    /// virtio-fs tag configured by the host (libkrun).
    pub tag: String,
    /// Absolute path inside the guest/container where this volume should be mounted.
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}

/// Configuration passed from host to guest via command-line JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestConfig {
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    pub workdir: Option<String>,
    #[serde(default)]
    pub tty: bool,
    pub vsock_port: u32,
    #[serde(default)]
    pub volumes: Vec<VolumeMount>,
}
