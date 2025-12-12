//! Userspace network stack for libkrun VMs.
//!
//! Provides NAT, DHCP, and DNS without external dependencies.

mod arp;
mod dhcp;
mod dns;
mod eth;
mod nat;
mod stack;

pub use stack::{VmNetwork, network_available};

/// Network constants.
pub const GATEWAY_IP: [u8; 4] = [192, 168, 127, 1];
pub const GUEST_IP: [u8; 4] = [192, 168, 127, 2];
pub const SUBNET_MASK: [u8; 4] = [255, 255, 255, 0];
pub const GATEWAY_MAC: [u8; 6] = [0x02, 0x52, 0x4f, 0x53, 0x53, 0x01];
pub const DEFAULT_MAC: [u8; 6] = [0x02, 0x52, 0x4f, 0x53, 0x53, 0x00];

/// Special IP for ross.host.internal that maps to host's localhost.
/// When the guest connects to this IP, NAT translates it to 127.0.0.1 on the host.
pub const HOST_IP: [u8; 4] = [192, 168, 127, 254];

/// Network features for virtio-net device.
pub const COMPAT_NET_FEATURES: u32 = (1 << 0)   // CSUM
    | (1 << 1)   // GUEST_CSUM
    | (1 << 7)   // GUEST_TSO4
    | (1 << 10)  // GUEST_UFO
    | (1 << 11)  // HOST_TSO4
    | (1 << 14); // HOST_UFO

/// Flag to send vfkit magic bytes.
pub const NET_FLAG_VFKIT: u32 = 1 << 0;
