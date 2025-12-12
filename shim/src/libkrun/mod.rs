//! libkrun-based container shim for macOS.
//!
//! This module implements the container shim using libkrun to run Linux
//! containers in a lightweight VM.

mod container;
mod rootfs;
mod shim;

#[cfg(all(feature = "libkrun", target_os = "macos"))]
mod krun;

pub use shim::KrunShim;
