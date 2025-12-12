//! Rootfs preparation for libkrun containers.
//!
//! Since libkrun doesn't support overlay mounts like runc, we need to
//! flatten all layers into a single directory.

use crate::rootfs as common_rootfs;
use crate::types::SnapshotMount;
use crate::ShimError;
use std::path::Path;
use tokio::fs;

/// The ross-init binary, compiled for Linux aarch64.
/// This is embedded at compile time from the guest crate build output.
#[cfg(all(feature = "libkrun", target_os = "macos"))]
const ROSS_INIT_BINARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../guest/target/release/ross-init"
));

/// Prepare rootfs from overlay mount specifications.
/// For libkrun, we copy all layers into a single directory.
pub async fn prepare_from_mounts(mounts: &[SnapshotMount], target: &Path) -> Result<(), ShimError> {
    fs::create_dir_all(target).await?;

    for mount in mounts {
        match mount.mount_type.as_str() {
            "overlay" => {
                let (lowerdirs, upperdir) = parse_overlay_options(&mount.options)?;

                for dir in lowerdirs.iter().rev() {
                    tracing::debug!("Copying lower layer: {}", dir);
                    copy_dir_contents(Path::new(dir), target).await?;
                }

                if let Some(upper) = upperdir {
                    tracing::debug!("Copying upper layer: {}", upper);
                    copy_dir_contents(Path::new(&upper), target).await?;
                }
            }
            "bind" => {
                tracing::debug!("Copying bind mount source: {}", mount.source);
                copy_dir_contents(Path::new(&mount.source), target).await?;
            }
            _ => {
                tracing::warn!("Unknown mount type: {}", mount.mount_type);
            }
        }
    }

    common_rootfs::ensure_essential_dirs(target).await?;

    // Install ross-init binary for interactive container support
    install_ross_init(target).await?;

    Ok(())
}

/// Install the ross-init binary into the rootfs.
/// This binary handles TTY/stdio forwarding inside the VM.
#[cfg(all(feature = "libkrun", target_os = "macos"))]
async fn install_ross_init(rootfs: &Path) -> Result<(), ShimError> {
    use std::os::unix::fs::PermissionsExt;

    let init_path = rootfs.join("ross-init");
    fs::write(&init_path, ROSS_INIT_BINARY).await?;

    // Make it executable (rwxr-xr-x = 0o755)
    let mut perms = fs::metadata(&init_path).await?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&init_path, perms).await?;

    tracing::debug!("Installed ross-init at {}", init_path.display());
    Ok(())
}

#[cfg(not(all(feature = "libkrun", target_os = "macos")))]
async fn install_ross_init(_rootfs: &Path) -> Result<(), ShimError> {
    Ok(())
}

fn parse_overlay_options(options: &[String]) -> Result<(Vec<String>, Option<String>), ShimError> {
    let mut lowerdirs = Vec::new();
    let mut upperdir = None;

    for opt in options {
        if let Some(dirs) = opt.strip_prefix("lowerdir=") {
            lowerdirs = dirs.split(':').map(String::from).collect();
        } else if let Some(dir) = opt.strip_prefix("upperdir=") {
            upperdir = Some(dir.to_string());
        }
    }

    Ok((lowerdirs, upperdir))
}

async fn copy_dir_contents(src: &Path, dst: &Path) -> Result<(), ShimError> {
    if !src.exists() {
        return Ok(());
    }

    let mut stack = vec![(src.to_path_buf(), std::path::PathBuf::new())];

    while let Some((current_src, relative)) = stack.pop() {
        let current_dst = dst.join(&relative);

        let mut entries = match fs::read_dir(&current_src).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };

        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if name_str.starts_with(".wh.") {
                if name_str == ".wh..wh..opq" {
                    if current_dst.exists() {
                        clear_directory(&current_dst).await?;
                    }
                } else {
                    let target_name = name_str.strip_prefix(".wh.").unwrap();
                    let target_path = current_dst.join(target_name);
                    if target_path.exists() {
                        if target_path.is_dir() {
                            fs::remove_dir_all(&target_path).await?;
                        } else {
                            fs::remove_file(&target_path).await?;
                        }
                    }
                }
                continue;
            }

            let src_path = entry.path();
            let dst_path = current_dst.join(&name);
            let file_type = entry.file_type().await?;

            if file_type.is_dir() {
                fs::create_dir_all(&dst_path).await?;
                stack.push((src_path, relative.join(&name)));
            } else if file_type.is_file() {
                if let Some(parent) = dst_path.parent() {
                    fs::create_dir_all(parent).await?;
                }
                fs::copy(&src_path, &dst_path).await?;
            } else if file_type.is_symlink() {
                let link_target = fs::read_link(&src_path).await?;
                if dst_path.exists() {
                    fs::remove_file(&dst_path).await?;
                }
                #[cfg(unix)]
                tokio::fs::symlink(&link_target, &dst_path).await?;
            }
        }
    }

    Ok(())
}

async fn clear_directory(dir: &Path) -> Result<(), ShimError> {
    let mut entries = fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_dir() {
            fs::remove_dir_all(&path).await?;
        } else {
            fs::remove_file(&path).await?;
        }
    }
    Ok(())
}
