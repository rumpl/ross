use crate::MountSpec;
use crate::error::MountError;
use std::path::Path;

#[cfg(target_os = "linux")]
use nix::mount::{MntFlags, MsFlags, mount, umount2};

/// Mount a filesystem based on the mount specification.
///
/// Supports:
/// - overlay: OverlayFS mount with lowerdir, upperdir, workdir options
/// - bind: Bind mount from source to target
#[cfg(target_os = "linux")]
pub fn mount_overlay(spec: &MountSpec, target: &Path) -> Result<(), MountError> {
    std::fs::create_dir_all(target)?;

    match spec.mount_type.as_str() {
        "overlay" => mount_overlay_fs(spec, target),
        "bind" => mount_bind(spec, target),
        other => Err(MountError::InvalidSpec(format!(
            "unsupported mount type: {}",
            other
        ))),
    }
}

#[cfg(not(target_os = "linux"))]
pub fn mount_overlay(_spec: &MountSpec, _target: &Path) -> Result<(), MountError> {
    Err(MountError::NotSupported(
        "overlay mounts are only supported on Linux".to_string(),
    ))
}

#[cfg(target_os = "linux")]
fn mount_overlay_fs(spec: &MountSpec, target: &Path) -> Result<(), MountError> {
    let options = spec.options.join(",");

    tracing::info!("Mounting overlay at {:?} with options: {}", target, options);

    mount(
        Some("overlay"),
        target,
        Some("overlay"),
        MsFlags::empty(),
        Some(options.as_str()),
    )
    .map_err(|e| MountError::MountFailed(format!("overlay mount failed: {}", e)))?;

    tracing::info!("Mounted overlay filesystem at {:?}", target);
    Ok(())
}

#[cfg(target_os = "linux")]
fn mount_bind(spec: &MountSpec, target: &Path) -> Result<(), MountError> {
    let source = Path::new(&spec.source);

    let mut flags = MsFlags::MS_BIND;

    for opt in &spec.options {
        match opt.as_str() {
            "ro" => flags |= MsFlags::MS_RDONLY,
            "rbind" => flags |= MsFlags::MS_REC,
            _ => {}
        }
    }

    tracing::info!(
        "Bind mounting {:?} to {:?} with flags {:?}, options {:?}",
        source,
        target,
        flags,
        spec.options
    );

    mount(Some(source), target, None::<&str>, flags, None::<&str>)
        .map_err(|e| MountError::MountFailed(format!("bind mount failed: {}", e)))?;

    // Apply read-only flag in a second mount call if needed
    if spec.options.iter().any(|o| o == "ro") {
        mount(
            None::<&str>,
            target,
            None::<&str>,
            MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY,
            None::<&str>,
        )
        .map_err(|e| MountError::MountFailed(format!("remount read-only failed: {}", e)))?;
    }

    tracing::info!("Bind mounted {:?} to {:?}", source, target);
    Ok(())
}

/// Unmount a filesystem at the given path.
#[cfg(target_os = "linux")]
pub fn unmount(target: &Path) -> Result<(), MountError> {
    tracing::debug!("Unmounting {:?}", target);

    umount2(target, MntFlags::MNT_DETACH)
        .map_err(|e| MountError::UnmountFailed(format!("unmount failed: {}", e)))?;

    tracing::info!("Unmounted {:?}", target);
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn unmount(_target: &Path) -> Result<(), MountError> {
    Err(MountError::NotSupported(
        "unmount is only supported on Linux".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mount_spec() {
        let spec = MountSpec::new(
            "overlay",
            "overlay",
            vec![
                "lowerdir=/lower".to_string(),
                "upperdir=/upper".to_string(),
                "workdir=/work".to_string(),
            ],
        );

        assert_eq!(spec.mount_type, "overlay");
        assert_eq!(spec.options.len(), 3);
    }
}
