//! libkrun VM execution.
//!
//! This module handles the actual libkrun VM creation and execution,
//! including the new TTY support via vsock.

use crate::ShimError;
use crate::guest_config::GuestConfig;
use std::ffi::CString;
use std::os::unix::io::RawFd;
use std::path::Path;

use super::network::{COMPAT_NET_FEATURES, NET_FLAG_VFKIT};

/// Network configuration for the VM.
#[derive(Clone, Debug)]
pub struct NetworkConfig {
    /// Path to gvproxy unix socket.
    pub socket_path: String,
    /// MAC address for the VM's network interface.
    pub mac: [u8; 6],
}

pub fn set_rlimits() {
    unsafe {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };

        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) == 0 {
            limit.rlim_cur = limit.rlim_max;
            libc::setrlimit(libc::RLIMIT_NOFILE, &limit);
        }
    }
}

/// Fix root directory permissions for libkrun.
/// Uses xattr to override stat for the container root.
pub fn fix_root_mode(rootfs: &Path) {
    let _ = std::process::Command::new("xattr")
        .args(["-w", "user.containers.override_stat", "0:0:0755"])
        .arg(rootfs)
        .output();
}

/// Wait for child process and return exit code.
pub fn wait_for_child(pid: libc::pid_t) -> i32 {
    let mut status: libc::c_int = 0;
    unsafe {
        libc::waitpid(pid, &mut status, 0);
    }
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        1
    }
}

/// Fork and run VM in child process (legacy non-interactive mode).
/// Returns (stdout_read_fd, child_pid) on success.
pub fn fork_and_run_vm(
    rootfs_path: &Path,
    exec_path: &str,
    argv: &[String],
    env: &[String],
    workdir: Option<&str>,
) -> Result<(RawFd, libc::pid_t), ShimError> {
    let mut stdout_pipe: [libc::c_int; 2] = [0, 0];
    if unsafe { libc::pipe(stdout_pipe.as_mut_ptr()) } != 0 {
        return Err(ShimError::RuntimeError("Failed to create pipe".to_string()));
    }

    let pid = unsafe { libc::fork() };

    if pid < 0 {
        return Err(ShimError::RuntimeError("Fork failed".to_string()));
    }

    if pid == 0 {
        unsafe {
            libc::close(stdout_pipe[0]);
            libc::dup2(stdout_pipe[1], libc::STDOUT_FILENO);
            libc::dup2(stdout_pipe[1], libc::STDERR_FILENO);
            libc::close(stdout_pipe[1]);
        }

        run_vm_inner(rootfs_path, exec_path, argv, env, workdir, None, None);
    }

    unsafe {
        libc::close(stdout_pipe[1]);
    }

    Ok((stdout_pipe[0], pid))
}

/// Fork and run VM with vsock for interactive I/O.
/// Returns child_pid on success.
#[allow(dead_code)]
pub fn fork_and_run_vm_interactive(
    rootfs_path: &Path,
    guest_config: &GuestConfig,
    vsock_port: u32,
) -> Result<libc::pid_t, ShimError> {
    fork_and_run_vm_interactive_with_network(rootfs_path, guest_config, vsock_port, None)
}

/// Fork and run VM with vsock for interactive I/O and optional network configuration.
/// Returns child_pid on success.
pub fn fork_and_run_vm_interactive_with_network(
    rootfs_path: &Path,
    guest_config: &GuestConfig,
    vsock_port: u32,
    network_config: Option<NetworkConfig>,
) -> Result<libc::pid_t, ShimError> {
    // Compute socket path before fork so both parent and child use the same path
    let socket_path = get_vsock_socket_path(vsock_port);

    // Write config to a file in the rootfs that ross-init can read
    let config_json = serde_json::to_string(guest_config)
        .map_err(|e| ShimError::RuntimeError(format!("Failed to serialize config: {}", e)))?;
    let config_path = rootfs_path.join(".ross-config.json");

    tracing::debug!(config_path = %config_path.display(), config_len = config_json.len(), "Writing guest config file");
    tracing::trace!(config = %config_json, "Guest config contents");

    std::fs::write(&config_path, &config_json)
        .map_err(|e| ShimError::RuntimeError(format!("Failed to write config file: {}", e)))?;

    // Verify the file was written
    match std::fs::read_to_string(&config_path) {
        Ok(contents) => {
            tracing::debug!(read_len = contents.len(), "Verified config file written");
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to verify config file");
        }
    }

    let pid = unsafe { libc::fork() };

    if pid < 0 {
        return Err(ShimError::RuntimeError("Fork failed".to_string()));
    }

    if pid == 0 {
        let exec_path = "/ross-init";
        let argv = vec![exec_path.to_string()];
        let env: Vec<String> = guest_config.env.clone();

        run_vm_inner(
            rootfs_path,
            exec_path,
            &argv,
            &env,
            guest_config.workdir.as_deref(),
            Some((vsock_port, socket_path)),
            network_config,
        );
    }

    Ok(pid)
}

fn run_vm_inner(
    rootfs_path: &Path,
    exec_path: &str,
    argv: &[String],
    env: &[String],
    workdir: Option<&str>,
    vsock_config: Option<(u32, String)>,
    network_config: Option<NetworkConfig>,
) -> ! {
    set_rlimits();

    let ctx_id = unsafe { krun_sys::krun_create_ctx() };
    if ctx_id < 0 {
        eprintln!("Failed to create context: {}", ctx_id);
        std::process::exit(1);
    }
    let ctx_id = ctx_id as u32;

    if unsafe { krun_sys::krun_set_vm_config(ctx_id, 2, 1100) } < 0 {
        eprintln!("Failed to set VM config");
        std::process::exit(1);
    }

    let root_cstr = CString::new(rootfs_path.to_string_lossy().as_bytes()).unwrap();
    if unsafe { krun_sys::krun_set_root(ctx_id, root_cstr.as_ptr()) } < 0 {
        eprintln!("Failed to set root");
        std::process::exit(1);
    }

    if let Some(wd) = workdir {
        let wd_cstr = CString::new(wd).unwrap();
        unsafe { krun_sys::krun_set_workdir(ctx_id, wd_cstr.as_ptr()) };
    }

    // Set up networking with gvproxy if configured.
    // This uses unixgram sockets with the vfkit protocol.
    // This must be done before krun_start_enter and disables TSI.
    if let Some(ref net_cfg) = network_config {
        eprintln!(
            "ross-shim: configuring network with gvproxy socket: {}",
            net_cfg.socket_path
        );
        let socket_cstr = CString::new(net_cfg.socket_path.as_bytes()).unwrap();
        let mut mac = net_cfg.mac;

        // Use krun_add_net_unixgram with vfkit flag for gvproxy compatibility
        let ret = unsafe {
            krun_sys::krun_add_net_unixgram(
                ctx_id,
                socket_cstr.as_ptr(), // path to gvproxy socket
                -1,                   // fd = -1 since we use path
                mac.as_mut_ptr(),     // MAC address
                COMPAT_NET_FEATURES,  // features
                NET_FLAG_VFKIT,       // send vfkit magic bytes
            )
        };

        if ret < 0 {
            eprintln!("Failed to configure network with gvproxy: {}", ret);
            std::process::exit(1);
        }
        eprintln!("ross-shim: network configured successfully (ret={})", ret);
    } else {
        eprintln!("ross-shim: no network config provided, using TSI");
    }

    // Set up vsock port mapping for TTY communication
    if let Some((port, socket_path)) = vsock_config {
        let socket_cstr = CString::new(socket_path.as_bytes()).unwrap();

        if unsafe { krun_sys::krun_add_vsock_port(ctx_id, port, socket_cstr.as_ptr()) } < 0 {
            eprintln!("Failed to add vsock port");
            std::process::exit(1);
        }
    }

    let exec_cstr = CString::new(exec_path).unwrap();
    let argv_cstrs: Vec<CString> = argv
        .iter()
        .map(|s| CString::new(s.as_bytes()).unwrap())
        .collect();
    let mut argv_ptrs: Vec<*const i8> = argv_cstrs.iter().map(|s| s.as_ptr()).collect();
    argv_ptrs.push(std::ptr::null());

    let env_cstrs: Vec<CString> = env
        .iter()
        .map(|s| CString::new(s.as_bytes()).unwrap())
        .collect();
    let mut env_ptrs: Vec<*const i8> = env_cstrs.iter().map(|s| s.as_ptr()).collect();
    env_ptrs.push(std::ptr::null());

    if unsafe {
        krun_sys::krun_set_exec(
            ctx_id,
            exec_cstr.as_ptr(),
            argv_ptrs.as_ptr(),
            env_ptrs.as_ptr(),
        )
    } < 0
    {
        eprintln!("Failed to set exec");
        std::process::exit(1);
    }

    let ret = unsafe { krun_sys::krun_start_enter(ctx_id) };
    std::process::exit(if ret == 0 { 0 } else { 1 });
}

/// Get the path to the Unix socket for vsock communication.
pub fn get_vsock_socket_path(port: u32) -> String {
    format!("/tmp/ross-vsock-{}.sock", port)
}
