//! Ross guest init - runs inside the VM to handle interactive containers.
//!
//! This binary is placed in the container rootfs and executed by libkrun.
//! It reads configuration from the environment or command line, then
//! spawns the requested command with proper TTY/pipe setup and forwards
//! I/O to the host via vsock.

pub mod protocol;
pub mod tty;

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

/// Configuration passed from host to guest.
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
