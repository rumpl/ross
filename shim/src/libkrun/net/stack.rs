//! Main network stack implementation.

use super::GATEWAY_IP;
use super::arp::handle_arp;
use super::dhcp::handle_dhcp;
use super::dns::{DnsForwarder, handle_dns};
use super::eth::{ETHERTYPE_ARP, ETHERTYPE_IPV4, IP_PROTO_ICMP, IP_PROTO_TCP, IP_PROTO_UDP};
use super::nat::{NatState, handle_icmp, handle_tcp, handle_udp, poll_nat_sockets};
use crate::ShimError;
use nix::sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, bind, socket};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

const VFKIT_MAGIC: [u8; 4] = *b"VFKT";

/// Userspace network stack for VM.
pub struct VmNetwork {
    socket_path: PathBuf,
    _server_fd: OwnedFd,
    shutdown: Arc<AtomicBool>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl VmNetwork {
    pub fn start(container_id: &str) -> Result<Self, ShimError> {
        let socket_path = PathBuf::from(format!("/tmp/ross-net-{}.sock", container_id));
        let _ = std::fs::remove_file(&socket_path);

        let server_fd = socket(
            AddressFamily::Unix,
            SockType::Datagram,
            SockFlag::empty(),
            None,
        )
        .map_err(|e| ShimError::RuntimeError(format!("socket: {}", e)))?;

        // Increase receive buffer size to prevent drops during high throughput
        unsafe {
            let buf_size: libc::c_int = 4 * 1024 * 1024; // 4MB
            libc::setsockopt(
                server_fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &buf_size as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }

        // Also increase send buffer size to reduce backpressure during bursts.
        unsafe {
            let buf_size: libc::c_int = 4 * 1024 * 1024; // 4MB
            libc::setsockopt(
                server_fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &buf_size as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }

        let addr = UnixAddr::new(&socket_path)
            .map_err(|e| ShimError::RuntimeError(format!("addr: {}", e)))?;

        bind(server_fd.as_raw_fd(), &addr)
            .map_err(|e| ShimError::RuntimeError(format!("bind: {}", e)))?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let fd = server_fd.as_raw_fd();

        let thread_handle = thread::spawn(move || run_stack(fd, shutdown_clone));

        tracing::info!(path = %socket_path.display(), "Network stack started");

        Ok(Self {
            socket_path,
            _server_fd: server_fd,
            shutdown,
            thread_handle: Some(thread_handle),
        })
    }

    pub fn socket_path(&self) -> &str {
        self.socket_path.to_str().unwrap_or("")
    }
}

impl Drop for VmNetwork {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(h) = self.thread_handle.take() {
            let _ = h.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

pub fn network_available() -> bool {
    true
}

fn run_stack(fd: i32, shutdown: Arc<AtomicBool>) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }

    // Wait for connection
    let mut buf = [0u8; 65535];
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }

        let mut src_addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        let mut src_len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;

        let n = unsafe {
            libc::recvfrom(
                fd,
                buf.as_mut_ptr() as *mut _,
                buf.len(),
                0,
                &mut src_addr as *mut _ as *mut _,
                &mut src_len,
            )
        };

        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock {
                thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }
            return;
        }

        if n >= 4 && buf[..4] == VFKIT_MAGIC {
            unsafe {
                libc::connect(fd, &src_addr as *const _ as *const _, src_len);
            }
            tracing::info!("VM connected");
            break;
        }

        thread::sleep(std::time::Duration::from_millis(10));
    }

    // Main loop - prioritize draining VM packets to prevent TX queue stalls
    let mut nat_state = NatState::new();
    let mut dns_forwarder: Option<DnsForwarder> = None;
    let mut pending_responses: Vec<Vec<u8>> = Vec::with_capacity(256);
    let mut nat_responses: Vec<Vec<u8>> = Vec::with_capacity(256);
    let mut idle_count = 0u32;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let mut received_any = false;

        // Phase 1: Drain ALL pending packets from VM as fast as possible
        // This is critical to prevent virtio TX queue stalls
        loop {
            let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), 0) };

            if n > 0 {
                received_any = true;
                let n = n as usize;
                if let Some(resp) = process_frame(&buf[..n], &mut nat_state, &mut dns_forwarder) {
                    pending_responses.push(resp);
                }
            } else if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::WouldBlock {
                    break; // No more packets
                } else if err.kind() == std::io::ErrorKind::ConnectionReset {
                    tracing::debug!("VM disconnected");
                    return;
                } else {
                    tracing::error!(error = %err, "recv error");
                    return;
                }
            } else {
                tracing::debug!("VM connection closed");
                return;
            }
        }

        // Phase 2: Send pending responses to VM
        for resp in pending_responses.drain(..) {
            send_packet(fd, &resp);
        }

        // Phase 3: Poll NAT sockets for data from remote servers
        poll_nat_sockets(&mut nat_state, &mut nat_responses);
        let sent_any = !nat_responses.is_empty();
        for resp in nat_responses.drain(..) {
            send_packet(fd, &resp);
        }

        // Only yield/sleep if we're truly idle
        if received_any || sent_any {
            idle_count = 0;
        } else {
            idle_count = idle_count.saturating_add(1);
            if idle_count > 10000 {
                // Been idle for a while, sleep briefly
                thread::sleep(std::time::Duration::from_micros(100));
            }
        }
    }

    tracing::debug!("Network stack stopped");
}

fn send_packet(fd: i32, data: &[u8]) {
    unsafe {
        libc::send(fd, data.as_ptr() as *const _, data.len(), 0);
    }
}

fn process_frame(
    frame: &[u8],
    nat_state: &mut NatState,
    dns_forwarder: &mut Option<DnsForwarder>,
) -> Option<Vec<u8>> {
    if frame.len() < 14 {
        return None;
    }

    let src_mac = &frame[6..12];
    let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    let payload = &frame[14..];

    match ethertype {
        ETHERTYPE_ARP => handle_arp(payload, src_mac),
        ETHERTYPE_IPV4 => process_ipv4(payload, src_mac, nat_state, dns_forwarder),
        _ => None,
    }
}

fn process_ipv4(
    payload: &[u8],
    src_mac: &[u8],
    nat_state: &mut NatState,
    dns_forwarder: &mut Option<DnsForwarder>,
) -> Option<Vec<u8>> {
    if payload.len() < 20 {
        return None;
    }

    let ihl = (payload[0] & 0x0f) as usize * 4;
    if payload.len() < ihl {
        return None;
    }

    let proto = payload[9];
    let src_ip = &payload[12..16];
    let dst_ip = &payload[16..20];
    let ip_payload = &payload[ihl..];

    match proto {
        IP_PROTO_ICMP => handle_icmp(ip_payload, src_mac, src_ip, dst_ip),
        IP_PROTO_UDP => {
            let dst_port = u16::from_be_bytes([ip_payload[2], ip_payload[3]]);
            if dst_port == 67 {
                handle_dhcp(&ip_payload[8..])
            } else if dst_port == 53 && dst_ip == GATEWAY_IP {
                let src_port = u16::from_be_bytes([ip_payload[0], ip_payload[1]]);
                handle_dns(&ip_payload[8..], src_mac, src_ip, src_port, dns_forwarder)
            } else {
                handle_udp(nat_state, ip_payload, src_mac, src_ip, dst_ip)
            }
        }
        IP_PROTO_TCP => handle_tcp(nat_state, ip_payload, src_mac, src_ip, dst_ip),
        _ => None,
    }
}
