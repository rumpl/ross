//! Network configuration for libkrun VMs on macOS.
//!
//! This module handles setting up networking using gvproxy (gvisor-tap-vsock)
//! which provides userspace networking compatible with vfkit/libkrun on macOS.
//!
//! gvproxy creates a virtual network with:
//! - Gateway at 192.168.127.1
//! - Host accessible at 192.168.127.254  
//! - DHCP for guest IP assignment
//! - DNS forwarding to host resolver

use crate::ShimError;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

/// Network features for virtio-net device.
const NET_FEATURE_CSUM: u32 = 1 << 0;
const NET_FEATURE_GUEST_CSUM: u32 = 1 << 1;
const NET_FEATURE_GUEST_TSO4: u32 = 1 << 7;
const NET_FEATURE_HOST_TSO4: u32 = 1 << 11;
const NET_FEATURE_HOST_UFO: u32 = 1 << 14;
const NET_FEATURE_GUEST_UFO: u32 = 1 << 10;

/// Compatible network features for gvproxy/passt.
pub const COMPAT_NET_FEATURES: u32 = NET_FEATURE_CSUM
    | NET_FEATURE_GUEST_CSUM
    | NET_FEATURE_GUEST_TSO4
    | NET_FEATURE_GUEST_UFO
    | NET_FEATURE_HOST_TSO4
    | NET_FEATURE_HOST_UFO;

/// Flag to send vfkit magic bytes after connection (required for gvproxy).
pub const NET_FLAG_VFKIT: u32 = 1 << 0;

/// Default MAC address for the VM's network interface.
pub const DEFAULT_MAC: [u8; 6] = [0x02, 0x52, 0x4f, 0x53, 0x53, 0x00]; // 02:52:4f:53:53:00 ("ROSS")

/// Manages a gvproxy process for VM networking on macOS.
///
/// gvproxy provides a userspace network stack that works without root privileges.
/// It creates a virtual network and handles NAT, DHCP, and DNS for the guest.
pub struct GvproxyNetwork {
    /// The gvproxy child process.
    child: Child,
    /// Path to the socket file for VM connection.
    socket_path: PathBuf,
}

impl GvproxyNetwork {
    /// Start gvproxy and wait for it to create its socket.
    ///
    /// The VM connects to gvproxy via a unix datagram socket using the vfkit protocol.
    pub fn start(container_id: &str) -> Result<Self, ShimError> {
        let socket_path = PathBuf::from(format!("/tmp/ross-gvproxy-{}.sock", container_id));

        // Remove old socket if it exists
        let _ = std::fs::remove_file(&socket_path);

        // Start gvproxy with vfkit socket mode
        // -listen-vfkit: unix datagram socket path for vfkit/libkrun connection
        // -debug: enable debug logging to help diagnose issues
        let child = Command::new("gvproxy")
            .args([
                "-listen-vfkit",
                &format!("unixgram://{}", socket_path.display()),
                "-debug",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                ShimError::RuntimeError(format!(
                    "Failed to start gvproxy (is it installed?): {}. \
                     Install with: brew install gvisor-tap-vsock",
                    e
                ))
            })?;

        // Wait for gvproxy to create the socket
        let mut attempts = 0;
        while !socket_path.exists() && attempts < 50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            attempts += 1;
        }

        if !socket_path.exists() {
            // Try to get stderr output from gvproxy to help debug
            let mut child = child;
            if let Some(mut stderr) = child.stderr.take() {
                use std::io::Read;
                let mut stderr_output = String::new();
                let _ = stderr.read_to_string(&mut stderr_output);
                if !stderr_output.is_empty() {
                    tracing::error!(stderr = %stderr_output, "gvproxy stderr output");
                }
            }
            return Err(ShimError::RuntimeError(format!(
                "gvproxy socket not created in time (waited {}ms, path: {})",
                attempts * 100,
                socket_path.display()
            )));
        }

        tracing::info!(socket_path = %socket_path.display(), "gvproxy started");

        Ok(Self { child, socket_path })
    }

    /// Get the socket path for configuring libkrun.
    pub fn socket_path(&self) -> &str {
        self.socket_path.to_str().unwrap_or("")
    }
}

impl Drop for GvproxyNetwork {
    fn drop(&mut self) {
        // Kill gvproxy process
        let _ = self.child.kill();
        let _ = self.child.wait();

        // Clean up socket
        let _ = std::fs::remove_file(&self.socket_path);

        tracing::debug!("gvproxy process cleaned up");
    }
}

/// Check if gvproxy is available on the system.
pub fn gvproxy_available() -> bool {
    Command::new("gvproxy")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
