//! Ross init - guest init process for interactive containers.
//!
//! This binary runs inside the VM as the init process. It reads a JSON
//! configuration specifying the command to run, then handles PTY/pipe
//! setup and I/O forwarding to the host via vsock.
//!
//! Usage:
//!   ross-init '<json-config>'
//!   ROSS_GUEST_CONFIG='<json-config>' ross-init

use ross_guest::{tty, GuestConfig};
use std::env;
use std::process::ExitCode;

const CONFIG_FILE_PATH: &str = "/.ross-config.json";

#[cfg(target_os = "linux")]
fn mount_volumes(config: &GuestConfig) -> ExitCode {
    use nix::mount::{mount, MsFlags};

    for v in &config.volumes {
        if v.tag.is_empty() || v.target.is_empty() {
            eprintln!("ross-init: skipping invalid volume entry: {:?}", v);
            continue;
        }
        if !v.target.starts_with('/') {
            eprintln!("ross-init: volume target must be absolute: {}", v.target);
            return ExitCode::from(1);
        }

        if let Err(e) = std::fs::create_dir_all(&v.target) {
            eprintln!("ross-init: failed to create mountpoint {}: {}", v.target, e);
            return ExitCode::from(1);
        }

        let mut flags = MsFlags::MS_NOSUID | MsFlags::MS_NODEV;
        if v.read_only {
            flags |= MsFlags::MS_RDONLY;
        }

        if let Err(e) = mount(
            Some(v.tag.as_str()),
            v.target.as_str(),
            Some("virtiofs"),
            flags,
            None::<&str>,
        ) {
            eprintln!(
                "ross-init: failed to mount virtiofs tag '{}' at '{}': {}",
                v.tag, v.target, e
            );
            return ExitCode::from(1);
        }

        eprintln!(
            "ross-init: mounted volume tag '{}' at '{}'{}",
            v.tag,
            v.target,
            if v.read_only { " (ro)" } else { "" }
        );
    }
    ExitCode::from(0)
}

#[cfg(not(target_os = "linux"))]
fn mount_volumes(config: &GuestConfig) -> ExitCode {
    // `ross-init` is meant to run inside the Linux guest. When we compile/test
    // this crate on the host (e.g. macOS), we just no-op unless volumes were
    // requested (which would indicate a misconfiguration).
    if config.volumes.is_empty() {
        ExitCode::from(0)
    } else {
        eprintln!("ross-init: volume mounts are only supported on Linux guests");
        ExitCode::from(1)
    }
}

fn setup_loopback() {
    // Bring up the loopback interface for localhost connectivity.
    // This mirrors what libkrun's init does.
    #[repr(C)]
    struct Ifreq {
        ifr_name: [libc::c_char; libc::IFNAMSIZ],
        ifr_flags: libc::c_short,
        _pad: [u8; 22],
    }

    unsafe {
        let sockfd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if sockfd >= 0 {
            let mut ifr: Ifreq = std::mem::zeroed();
            ifr.ifr_name[0] = b'l' as libc::c_char;
            ifr.ifr_name[1] = b'o' as libc::c_char;
            ifr.ifr_flags = libc::IFF_UP as libc::c_short;

            // SIOCSIFFLAGS = 0x8914 on Linux
            nix::libc::ioctl(sockfd, 0x8914, &ifr);
            libc::close(sockfd);
        }
    }
}

fn setup_eth0() {
    // Bring up eth0 interface if it exists (used when gvproxy/passt networking is enabled).
    // The actual IP configuration will be done via DHCP.
    #[repr(C)]
    struct Ifreq {
        ifr_name: [libc::c_char; libc::IFNAMSIZ],
        ifr_flags: libc::c_short,
        _pad: [u8; 22],
    }

    unsafe {
        let sockfd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if sockfd >= 0 {
            let mut ifr: Ifreq = std::mem::zeroed();
            // Set interface name to "eth0"
            ifr.ifr_name[0] = b'e' as libc::c_char;
            ifr.ifr_name[1] = b't' as libc::c_char;
            ifr.ifr_name[2] = b'h' as libc::c_char;
            ifr.ifr_name[3] = b'0' as libc::c_char;
            ifr.ifr_flags = libc::IFF_UP as libc::c_short;

            // SIOCSIFFLAGS = 0x8914 on Linux
            let ret = nix::libc::ioctl(sockfd, 0x8914, &ifr);
            if ret == 0 {
                eprintln!("ross-init: eth0 interface brought up");
            }
            libc::close(sockfd);
        }
    }
}

fn run_dhcp_client() {
    // Try to run a DHCP client to get an IP address from gvproxy/passt.
    // Both provide built-in DHCP servers.
    // Try common DHCP client locations.
    let dhcp_clients = [
        ("/sbin/dhclient", vec!["-v", "eth0"]),
        ("/sbin/udhcpc", vec!["-i", "eth0", "-f", "-q"]),
        ("/usr/sbin/dhclient", vec!["-v", "eth0"]),
        ("/usr/bin/dhcpcd", vec!["-4", "-q", "eth0"]),
    ];

    for (client, args) in &dhcp_clients {
        if std::path::Path::new(client).exists() {
            eprintln!("ross-init: running DHCP client: {} {:?}", client, args);
            match std::process::Command::new(client).args(args).status() {
                Ok(status) if status.success() => {
                    eprintln!("ross-init: DHCP client succeeded");
                    return;
                }
                Ok(status) => {
                    eprintln!("ross-init: DHCP client exited with: {}", status);
                }
                Err(e) => {
                    eprintln!("ross-init: DHCP client failed: {}", e);
                }
            }
        }
    }

    eprintln!("ross-init: no DHCP client found, network may not be configured");
}

#[cfg(target_os = "linux")]
fn write_sysctl(path: &str, value: &str) {
    // Best-effort: these are performance knobs; failure shouldn't prevent booting.
    if let Err(e) = std::fs::write(path, value) {
        eprintln!(
            "ross-init: sysctl write failed: {} = {} ({})",
            path,
            value.trim(),
            e
        );
    }
}

#[cfg(target_os = "linux")]
fn tune_tcp_buffers() {
    // Make high-bandwidth localhost/host networking fast without requiring user flags like:
    //   iperf -w 2M ...
    //
    // iperf2 uses SO_SNDBUF/SO_RCVBUF; if net.core.{wmem,rmem}_max are small, iperf prints:
    //   "TCP window size: 416 KByte (WARNING: requested 2.00 MByte)"
    //
    // These values are intentionally generous for VM/host-local traffic.
    write_sysctl("/proc/sys/net/core/rmem_max", "134217728\n"); // 128 MiB
    write_sysctl("/proc/sys/net/core/wmem_max", "134217728\n"); // 128 MiB
                                                                // Raise defaults so tools like iperf2 don't need `-w` to reach high throughput.
                                                                // (Linux may still clamp/double these internally.)
    write_sysctl("/proc/sys/net/core/rmem_default", "4194304\n"); // 4 MiB
    write_sysctl("/proc/sys/net/core/wmem_default", "4194304\n"); // 4 MiB

    // Enable autotuning and allow large windows.
    write_sysctl("/proc/sys/net/ipv4/tcp_window_scaling", "1\n");
    write_sysctl("/proc/sys/net/ipv4/tcp_moderate_rcvbuf", "1\n");

    // min default max (bytes)
    // Raise the "default" (middle) values substantially; the max stays high for `-w` requests.
    write_sysctl("/proc/sys/net/ipv4/tcp_rmem", "4096 4194304 134217728\n");
    write_sysctl("/proc/sys/net/ipv4/tcp_wmem", "4096 4194304 134217728\n");

    // Avoid throughput drop after brief idle periods (common in bursty workloads).
    write_sysctl("/proc/sys/net/ipv4/tcp_slow_start_after_idle", "0\n");
}

#[cfg(not(target_os = "linux"))]
fn tune_tcp_buffers() {}

fn main() -> ExitCode {
    // Set up loopback interface before anything else
    setup_loopback();

    // Try to set up eth0 and get IP via DHCP (for gvproxy/passt networking)
    setup_eth0();
    run_dhcp_client();
    tune_tcp_buffers();

    eprintln!("ross-init: starting");
    eprintln!("ross-init: args = {:?}", env::args().collect::<Vec<_>>());

    // Check if config file exists
    match std::fs::metadata(CONFIG_FILE_PATH) {
        Ok(m) => eprintln!("ross-init: config file exists, size = {}", m.len()),
        Err(e) => eprintln!("ross-init: config file check: {}", e),
    }

    // Try to read it
    match std::fs::read_to_string(CONFIG_FILE_PATH) {
        Ok(s) => eprintln!(
            "ross-init: config file contents ({} bytes): {:?}",
            s.len(),
            &s[..s.len().min(100)]
        ),
        Err(e) => eprintln!("ross-init: failed to read config file: {}", e),
    }

    // Check env var
    match env::var("ROSS_GUEST_CONFIG") {
        Ok(s) => eprintln!("ross-init: env var set, len = {}", s.len()),
        Err(_) => eprintln!("ross-init: env var not set"),
    }

    // Read config from: 1) command line (if it looks like JSON), 2) env var, 3) config file
    // Skip argv[1] if it looks like a path (starts with /)
    let config_json = env::args()
        .nth(1)
        .filter(|arg| arg.starts_with('{'))
        .or_else(|| env::var("ROSS_GUEST_CONFIG").ok())
        .or_else(|| std::fs::read_to_string(CONFIG_FILE_PATH).ok());

    let config_json = match config_json {
        Some(json) => json,
        None => {
            eprintln!("ross-init: no configuration provided");
            eprintln!("Usage: ross-init '<json-config>'");
            eprintln!("   or: ROSS_GUEST_CONFIG='<json>' ross-init");
            eprintln!("   or: place config at {}", CONFIG_FILE_PATH);
            return ExitCode::from(1);
        }
    };

    let config: GuestConfig = match serde_json::from_str(&config_json) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ross-init: failed to parse config: {}", e);
            eprintln!(
                "ross-init: config_json len = {}, first 200 chars: {:?}",
                config_json.len(),
                &config_json[..config_json.len().min(200)]
            );
            return ExitCode::from(1);
        }
    };

    // Validate config
    if config.command.is_empty() {
        eprintln!("ross-init: command is empty");
        return ExitCode::from(1);
    }

    if config.vsock_port == 0 {
        eprintln!("ross-init: vsock_port is required for interactive mode");
        return ExitCode::from(1);
    }

    // Mount requested virtio-fs volumes before starting the workload
    let mount_status = mount_volumes(&config);
    if mount_status != ExitCode::from(0) {
        return mount_status;
    }

    // Run the command
    match tty::run_guest_command(&config) {
        Ok(exit_code) => ExitCode::from(exit_code as u8),
        Err(e) => {
            eprintln!("ross-init: error running command: {}", e);
            ExitCode::from(1)
        }
    }
}
