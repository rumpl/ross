//! Root filesystem preparation for libkrun VMs.
//!
//! This module provides functionality to prepare a root filesystem from OCI image layers.
//! Unlike traditional container runtimes that use overlayfs, libkrun requires a single
//! directory containing the complete filesystem.

use crate::error::ShimError;
use flate2::read::GzDecoder;
use std::path::Path;
use tar::Archive;
use tokio::fs;

/// Extracts a gzipped tar layer into the target directory.
///
/// Handles OCI whiteout files (.wh.*) to properly delete files from lower layers.
pub fn extract_layer(layer_data: &[u8], target_dir: &Path) -> Result<(), ShimError> {
    let decoder = GzDecoder::new(layer_data);
    let mut archive = Archive::new(decoder);
    archive.set_preserve_permissions(true);
    archive.set_overwrite(true);

    for entry in archive
        .entries()
        .map_err(|e| ShimError::BundlePreparationFailed(format!("failed to read tar: {}", e)))?
    {
        let mut entry = entry.map_err(|e| {
            ShimError::BundlePreparationFailed(format!("failed to read tar entry: {}", e))
        })?;

        let path = entry.path().map_err(|e| {
            ShimError::BundlePreparationFailed(format!("failed to get entry path: {}", e))
        })?;

        // Handle OCI whiteout files
        if let Some(name) = path.file_name() {
            let name_str = name.to_string_lossy();

            // .wh..wh..opq marks the directory as opaque (delete all contents from lower layers)
            if name_str == ".wh..wh..opq" {
                let parent = target_dir.join(path.parent().unwrap_or(Path::new("")));
                if parent.exists() && parent.is_dir() {
                    clear_directory(&parent)?;
                }
                continue;
            }

            // .wh.<name> marks <name> as deleted
            if name_str.starts_with(".wh.") {
                let original_name = name_str.strip_prefix(".wh.").unwrap();
                let whiteout_target = target_dir
                    .join(path.parent().unwrap_or(Path::new("")))
                    .join(original_name);

                if whiteout_target.exists() {
                    if whiteout_target.is_dir() {
                        std::fs::remove_dir_all(&whiteout_target).map_err(|e| {
                            ShimError::BundlePreparationFailed(format!(
                                "failed to remove whiteout dir: {}",
                                e
                            ))
                        })?;
                    } else {
                        std::fs::remove_file(&whiteout_target).map_err(|e| {
                            ShimError::BundlePreparationFailed(format!(
                                "failed to remove whiteout file: {}",
                                e
                            ))
                        })?;
                    }
                }
                continue;
            }
        }

        entry.unpack_in(target_dir).map_err(|e| {
            ShimError::BundlePreparationFailed(format!("failed to unpack: {}", e))
        })?;
    }

    Ok(())
}

/// Clears all contents of a directory without removing the directory itself.
fn clear_directory(dir: &Path) -> Result<(), ShimError> {
    for entry in std::fs::read_dir(dir).map_err(|e| {
        ShimError::BundlePreparationFailed(format!("failed to read directory: {}", e))
    })? {
        let entry = entry.map_err(|e| {
            ShimError::BundlePreparationFailed(format!("failed to read entry: {}", e))
        })?;
        let path = entry.path();

        if path.is_dir() {
            std::fs::remove_dir_all(&path).map_err(|e| {
                ShimError::BundlePreparationFailed(format!("failed to remove dir: {}", e))
            })?;
        } else {
            std::fs::remove_file(&path).map_err(|e| {
                ShimError::BundlePreparationFailed(format!("failed to remove file: {}", e))
            })?;
        }
    }
    Ok(())
}

/// Prepares a root filesystem by extracting all layers in order.
///
/// This creates a single merged directory suitable for use with libkrun's `krun_set_root()`.
///
/// # Arguments
/// * `layers` - Iterator of (digest, layer_data) tuples, in order from bottom to top
/// * `target_dir` - Directory where the rootfs will be created
pub async fn prepare_rootfs<'a, I>(layers: I, target_dir: &Path) -> Result<(), ShimError>
where
    I: IntoIterator<Item = (&'a str, &'a [u8])>,
{
    fs::create_dir_all(target_dir).await?;

    for (digest, layer_data) in layers {
        tracing::debug!(digest = %digest, "Extracting layer");
        extract_layer(layer_data, target_dir)?;
    }

    // Ensure essential directories exist
    ensure_essential_dirs(target_dir).await?;

    Ok(())
}

/// Ensures essential Linux directories exist in the rootfs.
pub async fn ensure_essential_dirs(rootfs: &Path) -> Result<(), ShimError> {
    let essential_dirs = [
        "dev",
        "proc",
        "sys",
        "tmp",
        "run",
        "etc",
        "var",
        "var/log",
        "var/tmp",
    ];

    for dir in essential_dirs {
        let path = rootfs.join(dir);
        if !path.exists() {
            fs::create_dir_all(&path).await?;
        }
    }

    // Ensure /etc/resolv.conf exists (even if empty)
    let resolv_conf = rootfs.join("etc/resolv.conf");
    if !resolv_conf.exists() {
        fs::write(&resolv_conf, "# Generated by ross\n").await?;
    }

    Ok(())
}

/// Creates a minimal rootfs for testing or bootstrapping.
///
/// This creates a basic filesystem structure with busybox-like layout.
pub async fn create_minimal_rootfs(target_dir: &Path) -> Result<(), ShimError> {
    fs::create_dir_all(target_dir).await?;

    let dirs = [
        "bin", "dev", "etc", "home", "lib", "proc", "root", "run", "sbin", "sys", "tmp", "usr",
        "usr/bin", "usr/lib", "usr/sbin", "var", "var/log", "var/run", "var/tmp",
    ];

    for dir in dirs {
        fs::create_dir_all(target_dir.join(dir)).await?;
    }

    // Create minimal /etc files
    fs::write(
        target_dir.join("etc/passwd"),
        "root:x:0:0:root:/root:/bin/sh\nnobody:x:65534:65534:nobody:/:/sbin/nologin\n",
    )
    .await?;

    fs::write(target_dir.join("etc/group"), "root:x:0:\nnogroup:x:65534:\n").await?;

    fs::write(target_dir.join("etc/hosts"), "127.0.0.1 localhost\n").await?;

    fs::write(target_dir.join("etc/resolv.conf"), "# Generated by ross\n").await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_create_minimal_rootfs() {
        let temp_dir = TempDir::new().unwrap();
        create_minimal_rootfs(temp_dir.path()).await.unwrap();

        assert!(temp_dir.path().join("bin").exists());
        assert!(temp_dir.path().join("etc/passwd").exists());
        assert!(temp_dir.path().join("etc/hosts").exists());
    }
}
