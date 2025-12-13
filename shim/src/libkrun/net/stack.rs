//! Main network stack implementation.

use super::GATEWAY_IP;
use super::arp::handle_arp;
use super::dhcp::handle_dhcp;
use super::dns::{DnsForwarder, handle_dns};
use super::eth::{ETHERTYPE_ARP, ETHERTYPE_IPV4, IP_PROTO_ICMP, IP_PROTO_TCP, IP_PROTO_UDP};
use super::nat::{NatState, handle_icmp, handle_tcp, handle_udp, poll_nat_sockets};
use super::ring_spsc::{PacketRef, SpscPacketRing};
use crate::ShimError;
use nix::sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, bind, socket};
use std::collections::VecDeque;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

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

        // Increase socket buffer sizes to maximum for high-throughput networking.
        // These buffers are critical for preventing packet drops during bursts.
        // macOS allows up to 8MB per socket by default, but we request larger
        // and let the kernel cap to the maximum allowed.
        unsafe {
            let buf_size: libc::c_int = 128 * 1024 * 1024; // Request 128MB (will be capped by kernel)
            libc::setsockopt(
                server_fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &buf_size as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
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

    // Default is single-threaded unless explicitly enabled.
    let workers = net_workers();
    if workers > 1 {
        run_stack_multi(fd, shutdown, workers);
    } else {
        run_stack_single(fd, shutdown);
    }
}

fn net_workers() -> usize {
    // Opt-in: allow scaling across cores for high-throughput benchmarks (iperf).
    // More workers generally means higher throughput but also higher CPU usage.
    //
    // Example:
    //   ROSS_NET_WORKERS=4 ross ...
    if let Ok(v) = std::env::var("ROSS_NET_WORKERS") {
        if let Ok(n) = v.parse::<usize>() {
            return n.max(1).min(32);
        }
    }
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendResult {
    Sent,
    WouldBlock,
    Failed,
}

fn run_stack_single(fd: i32, shutdown: Arc<AtomicBool>) {
    // Main loop - prioritize draining VM packets to prevent TX queue stalls
    let mut nat_state = NatState::new();
    let mut dns_forwarder: Option<DnsForwarder> = None;
    let mut pending_responses: Vec<Vec<u8>> = Vec::with_capacity(512);
    let mut nat_responses: Vec<Vec<u8>> = Vec::with_capacity(512);
    let mut buf = [0u8; 65535];
    // Outbox of packets waiting for VM socket to become writable.
    let mut outbox: VecDeque<Vec<u8>> = VecDeque::with_capacity(2048);
    let mut idle_count = 0u32;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        // Phase 0: Flush any queued packets to the VM (never block RX).
        flush_outbox_nowait(fd, &mut outbox);

        let mut received_any = false;

        // Phase 1: Drain ALL pending packets from VM as fast as possible
        // This is critical to prevent virtio TX queue stalls.
        // Process in batches of up to 64 packets before checking outbox,
        // to balance RX throughput with TX responsiveness.
        let mut rx_batch = 0;
        loop {
            let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), 0) };

            if n > 0 {
                received_any = true;
                rx_batch += 1;
                let n = n as usize;
                if let Some(resp) = process_frame(&buf[..n], &mut nat_state, &mut dns_forwarder) {
                    pending_responses.push(resp);
                }
                // Periodically flush to keep TX moving
                if rx_batch >= 64 && !pending_responses.is_empty() {
                    for resp in pending_responses.drain(..) {
                        queue_or_send_nowait(fd, &mut outbox, resp);
                    }
                    flush_outbox_nowait(fd, &mut outbox);
                    rx_batch = 0;
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
            queue_or_send_nowait(fd, &mut outbox, resp);
        }

        // Phase 3: Poll NAT sockets for data from remote servers
        poll_nat_sockets(&mut nat_state, &mut nat_responses);
        let sent_any = !nat_responses.is_empty();
        for resp in nat_responses.drain(..) {
            queue_or_send_nowait(fd, &mut outbox, resp);
        }

        // Adaptive idle: spin briefly, then yield, then sleep
        // This reduces latency for bursty traffic while saving CPU during idle periods
        if received_any || sent_any {
            idle_count = 0;
        } else {
            idle_count = idle_count.saturating_add(1);
            if idle_count > 50000 {
                // Been idle for a while, sleep briefly
                thread::sleep(std::time::Duration::from_micros(100));
            } else if idle_count > 1000 {
                // Moderately idle - yield to other threads
                thread::yield_now();
            }
            // Below 1000 iterations: pure spin for lowest latency
        }
    }

    tracing::debug!("Network stack stopped");
}

fn run_stack_multi(fd: i32, shutdown: Arc<AtomicBool>, workers: usize) {
    tracing::info!(workers, "Network stack running in multi-threaded mode");
    run_stack_multi_lockfree(fd, shutdown, workers);
}

fn run_stack_multi_lockfree(fd: i32, shutdown: Arc<AtomicBool>, workers: usize) {
    tracing::info!(workers, "Multi-threaded lock-free mode");

    let rx_rings: Vec<Arc<SpscPacketRing>> = (0..workers)
        .map(|_| Arc::new(SpscPacketRing::new()))
        .collect();
    let tx_rings: Vec<Arc<SpscPacketRing>> = (0..workers)
        .map(|_| Arc::new(SpscPacketRing::new()))
        .collect();

    // Spawn workers.
    let mut handles = Vec::with_capacity(workers);
    for i in 0..workers {
        let rx = rx_rings[i].clone();
        let tx = tx_rings[i].clone();
        let shutdown = shutdown.clone();
        let h = thread::Builder::new()
            .name(format!("ross-net-worker-{}", i))
            .stack_size(4 * 1024 * 1024)
            .spawn(move || net_worker_loop_lockfree(fd, rx, tx, shutdown, false))
            .expect("spawn net worker");
        handles.push(h);
    }

    // Dedicated TX thread: drains TX rings and performs blocking sends.
    let tx_handle = {
        let shutdown = shutdown.clone();
        let tx_rings = tx_rings.clone();
        Some(
            thread::Builder::new()
                .name("ross-net-tx".to_string())
                .stack_size(2 * 1024 * 1024)
                .spawn(move || tx_sender_loop_lockfree(fd, tx_rings, shutdown))
                .expect("spawn net tx"),
        )
    };

    // Main thread: VM RX -> dispatch to workers.
    let mut buf = vec![0u8; 65535];
    let mut idle_count = 0u32;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let mut received_any = false;
        loop {
            let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), 0) };
            if n > 0 {
                received_any = true;
                let n = n as usize;
                let shard = shard_for_frame(&buf[..n], workers);
                // CRITICAL: never spin-wait on ring capacity here; it stalls VM draining
                // and triggers virtio-net TX watchdog timeouts. Drop instead.
                let _ = rx_rings[shard].push(&buf[..n]);
            } else if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::WouldBlock {
                    break;
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

        if received_any {
            idle_count = 0;
        } else {
            idle_count = idle_count.saturating_add(1);
            if idle_count > 10000 {
                thread::sleep(Duration::from_micros(50));
            } else {
                thread::yield_now();
            }
        }
    }

    for h in handles {
        let _ = h.join();
    }
    if let Some(h) = tx_handle {
        let _ = h.join();
    }
    tracing::debug!("Network stack stopped");
}

fn net_worker_loop_lockfree(
    fd: i32,
    rx: Arc<SpscPacketRing>,
    tx: Arc<SpscPacketRing>,
    shutdown: Arc<AtomicBool>,
    direct_send: bool,
) {
    let mut nat_state = NatState::new();
    let mut dns_forwarder: Option<DnsForwarder> = None;
    let mut nat_responses: Vec<Vec<u8>> = Vec::with_capacity(256);
    let mut outbox: VecDeque<Vec<u8>> = VecDeque::with_capacity(1024);
    let mut pending_tx: VecDeque<Vec<u8>> = VecDeque::with_capacity(1024);
    let mut idle_count = 0u32;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let mut did_work = false;

        // Flush pending responses to TX ring first (never spin).
        while let Some(front) = pending_tx.front() {
            if tx.push(front) {
                let _ = pending_tx.pop_front();
            } else {
                break;
            }
        }

        if direct_send {
            flush_outbox_nowait(fd, &mut outbox);
        }

        while let Some(pkt) = rx.pop_ref() {
            did_work = true;
            if let Some(resp) = process_frame(&pkt, &mut nat_state, &mut dns_forwarder) {
                if direct_send {
                    queue_or_send_nowait(fd, &mut outbox, resp);
                } else {
                    if !tx.push(&resp) && pending_tx.len() < 4096 {
                        pending_tx.push_back(resp);
                    }
                }
            }
        }

        poll_nat_sockets(&mut nat_state, &mut nat_responses);
        if !nat_responses.is_empty() {
            did_work = true;
            for resp in nat_responses.drain(..) {
                if direct_send {
                    queue_or_send_nowait(fd, &mut outbox, resp);
                } else {
                    if !tx.push(&resp) && pending_tx.len() < 4096 {
                        pending_tx.push_back(resp);
                    }
                }
            }
        }

        if did_work {
            idle_count = 0;
        } else {
            idle_count = idle_count.saturating_add(1);
            if idle_count > 10000 {
                thread::sleep(Duration::from_micros(50));
            } else {
                thread::yield_now();
            }
        }
    }
}

fn tx_sender_loop_lockfree(fd: i32, tx_rings: Vec<Arc<SpscPacketRing>>, shutdown: Arc<AtomicBool>) {
    // Send directly from ring storage (no memcpy) and only poll when we hit EAGAIN.
    let mut idle_count = 0u32;
    
    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let mut did_work = false;

        // Round-robin through rings to avoid starvation
        for ring in tx_rings.iter() {
            // Drain up to 64 packets per ring per iteration for better batching
            for _ in 0..64 {
                let Some(pkt) = ring.pop_ref() else { break };
                did_work = true;
                if !send_packet_ref(fd, pkt) {
                    // Permanent send failure; keep looping to avoid deadlock.
                    break;
                }
            }
        }

        if did_work {
            idle_count = 0;
        } else {
            idle_count = idle_count.saturating_add(1);
            if idle_count > 10000 {
                thread::sleep(Duration::from_micros(50));
            } else if idle_count > 100 {
                thread::yield_now();
            }
            // Pure spin for < 100 iterations
        }
    }
}

fn send_packet_ref(fd: i32, pkt: PacketRef<'_>) -> bool {
    // Keep pkt alive across retries; it is consumed when dropped.
    let mut pkt = Some(pkt);
    loop {
        let p = pkt.as_ref().unwrap();
        let rc = unsafe { libc::send(fd, p.as_ptr() as *const _, p.len(), 0) };
        if rc >= 0 {
            // Consume packet now.
            drop(pkt.take());
            return true;
        }

        let err = std::io::Error::last_os_error();
        match err.kind() {
            std::io::ErrorKind::Interrupted => continue,
            std::io::ErrorKind::WouldBlock => {
                let mut pfd = libc::pollfd {
                    fd,
                    events: libc::POLLOUT,
                    revents: 0,
                };
                loop {
                    let prc = unsafe { libc::poll(&mut pfd as *mut libc::pollfd, 1, 1) };
                    if prc > 0 {
                        break;
                    }
                    if prc == 0 {
                        continue;
                    }
                    let perr = std::io::Error::last_os_error();
                    if perr.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    tracing::debug!(error = %perr, "poll(POLLOUT) failed");
                    return false;
                }
            }
            _ => {
                tracing::debug!(error = %err, "send failed");
                return false;
            }
        }
    }
}

fn send_packet_nowait(fd: i32, data: &[u8]) -> SendResult {
    // Unix datagram send is atomic: it's either sent or it fails.
    // Under load, the socket can apply backpressure (EAGAIN). In the RX/drain loop,
    // we must NOT block waiting for POLLOUT; queue instead.
    loop {
        let rc = unsafe { libc::send(fd, data.as_ptr() as *const _, data.len(), 0) };
        if rc >= 0 {
            return SendResult::Sent;
        }
        let err = std::io::Error::last_os_error();
        match err.kind() {
            std::io::ErrorKind::Interrupted => continue,
            std::io::ErrorKind::WouldBlock => return SendResult::WouldBlock,
            _ => {
                tracing::debug!(error = %err, "send_packet_nowait failed");
                return SendResult::Failed;
            }
        }
    }
}

fn queue_or_send_nowait(fd: i32, outbox: &mut VecDeque<Vec<u8>>, pkt: Vec<u8>) {
    // Large outbox to handle burst traffic - 16K packets at 1500 bytes = 24MB max
    const OUTBOX_MAX: usize = 16384;
    if outbox.is_empty() {
        match send_packet_nowait(fd, &pkt) {
            SendResult::Sent => return,
            SendResult::WouldBlock => {
                if outbox.len() < OUTBOX_MAX {
                    outbox.push_back(pkt);
                }
                return;
            }
            SendResult::Failed => return,
        }
    } else if outbox.len() < OUTBOX_MAX {
        outbox.push_back(pkt);
    }
    // Drop packet if outbox is full (better than stalling)
}

fn flush_outbox_nowait(fd: i32, outbox: &mut VecDeque<Vec<u8>>) {
    // Use sendmmsg to batch multiple packets in a single syscall when available
    #[cfg(target_os = "linux")]
    {
        flush_outbox_sendmmsg(fd, outbox);
    }
    #[cfg(not(target_os = "linux"))]
    {
        // macOS doesn't have sendmmsg, fall back to individual sends
        while let Some(pkt) = outbox.front() {
            match send_packet_nowait(fd, pkt) {
                SendResult::Sent => {
                    let _ = outbox.pop_front();
                }
                SendResult::WouldBlock => break,
                SendResult::Failed => {
                    let _ = outbox.pop_front();
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn flush_outbox_sendmmsg(fd: i32, outbox: &mut VecDeque<Vec<u8>>) {
    const MAX_BATCH: usize = 64;
    
    while !outbox.is_empty() {
        let batch_size = outbox.len().min(MAX_BATCH);
        if batch_size == 0 {
            break;
        }
        
        // Build iovec and mmsghdr arrays
        let mut iovecs: [libc::iovec; MAX_BATCH] = unsafe { std::mem::zeroed() };
        let mut msghdrs: [libc::mmsghdr; MAX_BATCH] = unsafe { std::mem::zeroed() };
        
        for (i, pkt) in outbox.iter().take(batch_size).enumerate() {
            iovecs[i] = libc::iovec {
                iov_base: pkt.as_ptr() as *mut _,
                iov_len: pkt.len(),
            };
            msghdrs[i].msg_hdr.msg_iov = &mut iovecs[i];
            msghdrs[i].msg_hdr.msg_iovlen = 1;
        }
        
        let sent = unsafe {
            libc::sendmmsg(fd, msghdrs.as_mut_ptr(), batch_size as u32, 0)
        };
        
        if sent <= 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                break;
            }
            // On error, try to at least drain one packet to avoid infinite loop
            let _ = outbox.pop_front();
            break;
        }
        
        // Remove sent packets
        for _ in 0..sent {
            let _ = outbox.pop_front();
        }
    }
}

#[inline]
fn shard_for_frame(frame: &[u8], workers: usize) -> usize {
    if workers <= 1 || frame.len() < 14 {
        return 0;
    }
    let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    if ethertype != ETHERTYPE_IPV4 || frame.len() < 14 + 20 {
        return 0;
    }

    let ip = &frame[14..];
    let ihl = (ip[0] & 0x0f) as usize * 4;
    if ihl < 20 || ip.len() < ihl {
        return 0;
    }

    let proto = ip[9];
    let src_ip = &ip[12..16];
    let dst_ip = &ip[16..20];
    let l4 = &ip[ihl..];

    let mut src_port = 0u16;
    let mut dst_port = 0u16;
    if (proto == IP_PROTO_TCP || proto == IP_PROTO_UDP) && l4.len() >= 4 {
        src_port = u16::from_be_bytes([l4[0], l4[1]]);
        dst_port = u16::from_be_bytes([l4[2], l4[3]]);
    }

    // Cheap hash; good enough to spread flows across workers.
    let mut h: u32 = (proto as u32).wrapping_mul(0x9e37_79b9);
    h ^= u32::from_be_bytes([src_ip[0], src_ip[1], src_ip[2], src_ip[3]]);
    h = h.rotate_left(13) ^ u32::from_be_bytes([dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3]]);
    // IMPORTANT: include `src_port` in the low bits so modulo small worker counts
    // (e.g. 8) actually distributes flows. Put dst_port in the high bits.
    let ports = (src_port as u32) | ((dst_port as u32) << 16);
    h = h.wrapping_mul(0x85eb_ca6b) ^ ports;
    (h as usize) % workers
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
