//! NAT for TCP and UDP connections.

use super::eth::{
    ETHERTYPE_IPV4, IP_PROTO_ICMP, IP_PROTO_TCP, IP_PROTO_UDP, build_eth_header, build_ip_header,
    checksum, tcp_udp_checksum,
};
use super::{GATEWAY_MAC, HOST_IP};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

// Max TCP payload we place in a single Ethernet+IPv4+TCP frame.
// Keep IP total length <= 1500 (typical MTU): 1500 - 20 (IP) - 20 (TCP) = 1460.
const MAX_SEGMENT_SIZE: usize = 1460;
// Large read buffer for batching: read up to 64KB from host sockets at once,
// then split into MSS-sized segments. This dramatically reduces syscall overhead.
const TCP_READ_BUFFER_SIZE: usize = 64 * 1024;
// Upper bound for in-flight bytes to guest (we also gate on the guest's advertised window).
// Keep this high to allow the TCP pipeline to stay full during high-throughput transfers.
const TCP_INFLIGHT_CAP: u32 = 4 * 1024 * 1024; // 4MiB - allows ~2.7ms of data at 10Gbps
const UDP_MAX_DATAGRAM: usize = 65535;
const OUR_WSCALE: u8 = 7; // advertise 128x window scale to guest (~8MiB effective at 65535)

// Socket buffer sizes for TCP connections
const TCP_SOCKET_SNDBUF: i32 = 4 * 1024 * 1024; // 4MB send buffer
const TCP_SOCKET_RCVBUF: i32 = 4 * 1024 * 1024; // 4MB receive buffer

/// Translate destination IP if it's the special host IP.
/// Returns (actual_ip, original_ip) where actual_ip is what we connect to
/// and original_ip is what we report back to the guest.
fn translate_host_ip(dst_ip: &[u8]) -> ([u8; 4], [u8; 4]) {
    let dst = [dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3]];
    if dst == HOST_IP {
        // Translate to localhost
        ([127, 0, 0, 1], dst)
    } else {
        (dst, dst)
    }
}

/// TCP connection state.
struct TcpNatEntry {
    stream: TcpStream,
    client_mac: [u8; 6],
    client_ip: [u8; 4],
    client_port: u16,
    remote_ip: [u8; 4],
    remote_port: u16,
    /// Our sequence number (next byte we'll send)
    our_seq: u32,
    /// Highest ACKed sequence number from guest
    acked_seq: u32,
    /// Next expected sequence number from guest
    expected_guest_seq: u32,
    /// Guest advertised receive window (unscaled).
    guest_window: u32,
    /// Guest window scale shift (as announced in SYN).
    guest_wscale: u8,
    last_active: Instant,
    /// Pending data to write to the remote server
    write_buffer: Vec<u8>,
    write_offset: usize,
}

impl TcpNatEntry {
    fn can_send(&self) -> bool {
        // Simple flow control: only send if we haven't sent too much unacked data
        let unacked = self.our_seq.wrapping_sub(self.acked_seq);
        // Guest's window field is scaled by the shift it announced in SYN.
        let guest_adv = self
            .guest_window
            .checked_shl(self.guest_wscale as u32)
            .unwrap_or(u32::MAX);
        let limit = guest_adv.min(TCP_INFLIGHT_CAP);
        unacked < limit
    }
}

/// UDP NAT entry.
struct UdpNatEntry {
    socket: UdpSocket,
    client_mac: [u8; 6],
    client_ip: [u8; 4],
    client_port: u16,
    last_active: Instant,
}

/// NAT state.
pub struct NatState {
    tcp: HashMap<([u8; 4], u16, u16), TcpNatEntry>,
    udp: HashMap<([u8; 4], u16, u16), UdpNatEntry>,
    // Reusable scratch buffers to avoid per-poll/per-packet stack allocations.
    udp_rx_buf: Vec<u8>,
    // Large read buffer to batch reads from host sockets
    tcp_rx_buf: Vec<u8>,
    tcp_keys_scratch: Vec<([u8; 4], u16, u16)>,
}

impl NatState {
    pub fn new() -> Self {
        Self {
            tcp: HashMap::new(),
            udp: HashMap::new(),
            udp_rx_buf: vec![0u8; UDP_MAX_DATAGRAM],
            tcp_rx_buf: vec![0u8; TCP_READ_BUFFER_SIZE],
            tcp_keys_scratch: Vec::with_capacity(64),
        }
    }
}

/// Handle ICMP packets.
pub fn handle_icmp(
    payload: &[u8],
    src_mac: &[u8],
    src_ip: &[u8],
    dst_ip: &[u8],
) -> Option<Vec<u8>> {
    if payload.len() < 8 || payload[0] != 8 {
        return None;
    }
    build_icmp_reply(src_mac, src_ip, dst_ip, payload)
}

fn build_icmp_reply(
    dst_mac: &[u8],
    dst_ip: &[u8],
    src_ip: &[u8],
    request: &[u8],
) -> Option<Vec<u8>> {
    let icmp_len = request.len();
    let total_len = 14 + 20 + icmp_len;

    let eth = build_eth_header(dst_mac, &GATEWAY_MAC, ETHERTYPE_IPV4);
    let ip = build_ip_header(src_ip, dst_ip, IP_PROTO_ICMP, icmp_len, 0);

    let mut response = Vec::with_capacity(total_len);
    response.extend_from_slice(&eth);
    response.extend_from_slice(&ip);
    response.extend_from_slice(request);

    // Flip echo request -> reply, recompute checksum in-place.
    let icmp_start = 14 + 20;
    response[icmp_start] = 0;
    response[icmp_start + 2..icmp_start + 4].copy_from_slice(&[0, 0]);
    let cksum = checksum(&response[icmp_start..icmp_start + icmp_len]);
    response[icmp_start + 2..icmp_start + 4].copy_from_slice(&cksum.to_be_bytes());

    Some(response)
}

/// Handle UDP packets.
pub fn handle_udp(
    state: &mut NatState,
    payload: &[u8],
    src_mac: &[u8],
    src_ip: &[u8],
    dst_ip: &[u8],
) -> Option<Vec<u8>> {
    if payload.len() < 8 {
        return None;
    }

    let src_port = u16::from_be_bytes([payload[0], payload[1]]);
    let dst_port = u16::from_be_bytes([payload[2], payload[3]]);
    let data = &payload[8..];

    // Translate HOST_IP to localhost
    let (actual_ip, original_ip) = translate_host_ip(dst_ip);

    // Key uses original IP so responses go back correctly
    let key = (original_ip, dst_port, src_port);

    let entry = state.udp.entry(key).or_insert_with(|| {
        let socket = UdpSocket::bind("0.0.0.0:0").expect("bind UDP");
        socket.set_nonblocking(true).ok();
        // Connect to actual IP (localhost for HOST_IP)
        let dst = SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(
                actual_ip[0],
                actual_ip[1],
                actual_ip[2],
                actual_ip[3],
            )),
            dst_port,
        );
        socket.connect(dst).ok();
        UdpNatEntry {
            socket,
            client_mac: [
                src_mac[0], src_mac[1], src_mac[2], src_mac[3], src_mac[4], src_mac[5],
            ],
            client_ip: [src_ip[0], src_ip[1], src_ip[2], src_ip[3]],
            client_port: src_port,
            last_active: Instant::now(),
        }
    });

    entry.last_active = Instant::now();
    let _ = entry.socket.send(data);

    if let Ok(len) = entry.socket.recv(&mut state.udp_rx_buf) {
        // Use original_ip in response so guest sees the IP it connected to
        return build_udp_response(
            &entry.client_mac,
            &entry.client_ip,
            entry.client_port,
            dst_port,
            &original_ip,
            &state.udp_rx_buf[..len],
        );
    }
    None
}

fn build_udp_response(
    dst_mac: &[u8],
    dst_ip: &[u8],
    dst_port: u16,
    src_port: u16,
    src_ip: &[u8],
    data: &[u8],
) -> Option<Vec<u8>> {
    let udp_len = 8 + data.len();
    let ip = build_ip_header(src_ip, dst_ip, IP_PROTO_UDP, udp_len, 0);
    let eth = build_eth_header(dst_mac, &GATEWAY_MAC, ETHERTYPE_IPV4);

    let mut response = Vec::with_capacity(14 + 20 + udp_len);
    response.extend_from_slice(&eth);
    response.extend_from_slice(&ip);

    // UDP header (checksum filled after payload copy).
    response.extend_from_slice(&src_port.to_be_bytes());
    response.extend_from_slice(&dst_port.to_be_bytes());
    response.extend_from_slice(&(udp_len as u16).to_be_bytes());
    response.extend_from_slice(&[0, 0]);
    response.extend_from_slice(data);

    let udp_start = 14 + 20;
    let udp_end = udp_start + udp_len;
    let cksum = tcp_udp_checksum(src_ip, dst_ip, IP_PROTO_UDP, &response[udp_start..udp_end]);
    response[udp_start + 6..udp_start + 8].copy_from_slice(&cksum.to_be_bytes());

    Some(response)
}

/// Handle TCP packets.
pub fn handle_tcp(
    state: &mut NatState,
    payload: &[u8],
    src_mac: &[u8],
    src_ip: &[u8],
    dst_ip: &[u8],
) -> Option<Vec<u8>> {
    if payload.len() < 20 {
        return None;
    }

    let src_port = u16::from_be_bytes([payload[0], payload[1]]);
    let dst_port = u16::from_be_bytes([payload[2], payload[3]]);
    let seq = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
    let ack = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);
    let data_offset = ((payload[12] >> 4) * 4) as usize;
    let flags = payload[13];
    let window = u16::from_be_bytes([payload[14], payload[15]]) as u32;

    let syn = flags & 0x02 != 0;
    let ack_flag = flags & 0x10 != 0;
    let fin = flags & 0x01 != 0;
    let rst = flags & 0x04 != 0;

    let data = if data_offset < payload.len() {
        &payload[data_offset..]
    } else {
        &[]
    };
    let key = (
        [dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3]],
        dst_port,
        src_port,
    );

    tracing::trace!(
        src_port,
        dst_port,
        seq,
        ack,
        syn,
        ack_flag,
        fin,
        rst,
        data_len = data.len(),
        "TCP rx"
    );

    if rst {
        state.tcp.remove(&key);
        return None;
    }

    // SYN - new connection
    if syn && !ack_flag {
        let opts = if data_offset > 20 && data_offset <= payload.len() {
            &payload[20..data_offset]
        } else {
            &[]
        };
        return handle_tcp_syn(
            state, key, src_mac, src_ip, dst_ip, src_port, dst_port, seq, opts,
        );
    }

    let entry = state.tcp.get_mut(&key)?;
    entry.last_active = Instant::now();
    // Track the guest advertised receive window (unscaled TCP header field).
    entry.guest_window = window.max(1024); // clamp away pathological 0/1 windows

    // Update acked_seq from guest's ACK
    if ack_flag && ack > entry.acked_seq {
        entry.acked_seq = ack;
    }

    // Handle retransmit
    if seq < entry.expected_guest_seq {
        return build_tcp_packet(
            &entry.client_mac,
            &entry.client_ip,
            entry.client_port,
            entry.remote_port,
            &entry.remote_ip,
            entry.our_seq,
            entry.expected_guest_seq,
            0x10,
            &[],
        );
    }

    // Out of order
    if seq > entry.expected_guest_seq && !data.is_empty() {
        return build_tcp_packet(
            &entry.client_mac,
            &entry.client_ip,
            entry.client_port,
            entry.remote_port,
            &entry.remote_ip,
            entry.our_seq,
            entry.expected_guest_seq,
            0x10,
            &[],
        );
    }

    // Process data from guest.
    // Fast path: if we have no pending buffered data, try to write directly to the remote stream
    // to avoid an extra userspace copy into write_buffer.
    if !data.is_empty() {
        if entry.write_offset == 0 && entry.write_buffer.is_empty() {
            match entry.stream.write(data) {
                Ok(0) => {
                    let resp = build_tcp_packet(
                        &entry.client_mac,
                        &entry.client_ip,
                        entry.client_port,
                        entry.remote_port,
                        &entry.remote_ip,
                        0,
                        0,
                        0x04,
                        &[],
                    );
                    state.tcp.remove(&key);
                    return resp;
                }
                Ok(n) if n == data.len() => {
                    // fully written, no buffering needed
                }
                Ok(n) => {
                    entry.write_buffer.extend_from_slice(&data[n..]);
                    entry.write_offset = 0;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    entry.write_buffer.extend_from_slice(data);
                    entry.write_offset = 0;
                }
                Err(e) => {
                    tracing::debug!(error = %e, "TCP write failed");
                    let resp = build_tcp_packet(
                        &entry.client_mac,
                        &entry.client_ip,
                        entry.client_port,
                        entry.remote_port,
                        &entry.remote_ip,
                        0,
                        0,
                        0x04,
                        &[],
                    );
                    state.tcp.remove(&key);
                    return resp;
                }
            }
        } else {
            // Slow path: buffer and flush.
            if entry.write_offset >= entry.write_buffer.len() {
                entry.write_buffer.clear();
                entry.write_offset = 0;
            }
            entry.write_buffer.extend_from_slice(data);
        }
        entry.expected_guest_seq = entry.expected_guest_seq.wrapping_add(data.len() as u32);
    }

    // Try to flush write buffer
    if entry.write_offset < entry.write_buffer.len() {
        match entry
            .stream
            .write(&entry.write_buffer[entry.write_offset..])
        {
            Ok(0) => {
                // Connection closed
                let resp = build_tcp_packet(
                    &entry.client_mac,
                    &entry.client_ip,
                    entry.client_port,
                    entry.remote_port,
                    &entry.remote_ip,
                    0,
                    0,
                    0x04,
                    &[],
                );
                state.tcp.remove(&key);
                return resp;
            }
            Ok(n) => {
                entry.write_offset = entry.write_offset.saturating_add(n);
                // Occasionally compact to avoid unbounded growth if we append a lot.
                if entry.write_offset > 64 * 1024
                    && entry.write_offset >= entry.write_buffer.len() / 2
                {
                    compact_write_buffer(entry);
                } else if entry.write_offset >= entry.write_buffer.len() {
                    entry.write_buffer.clear();
                    entry.write_offset = 0;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Can't write now, will retry later
            }
            Err(e) => {
                tracing::debug!(error = %e, "TCP write failed");
                let resp = build_tcp_packet(
                    &entry.client_mac,
                    &entry.client_ip,
                    entry.client_port,
                    entry.remote_port,
                    &entry.remote_ip,
                    0,
                    0,
                    0x04,
                    &[],
                );
                state.tcp.remove(&key);
                return resp;
            }
        }
    }

    // FIN
    if fin {
        entry.expected_guest_seq = entry.expected_guest_seq.wrapping_add(1);
        let resp = build_tcp_packet(
            &entry.client_mac,
            &entry.client_ip,
            entry.client_port,
            entry.remote_port,
            &entry.remote_ip,
            entry.our_seq,
            entry.expected_guest_seq,
            0x11,
            &[],
        );
        state.tcp.remove(&key);
        return resp;
    }

    // Try to send data to guest if we have window space
    // Read up to MAX_SEGMENT_SIZE here since we can only return one packet.
    // The bulk of data transfer happens in poll_nat_sockets with batch reads.
    if entry.can_send() {
        // Use a stack buffer for quick inline reads (avoid indexing the large heap buffer)
        let mut quick_buf = [0u8; MAX_SEGMENT_SIZE];
        match entry.stream.read(&mut quick_buf) {
            Ok(0) => {
                let resp = build_tcp_packet(
                    &entry.client_mac,
                    &entry.client_ip,
                    entry.client_port,
                    entry.remote_port,
                    &entry.remote_ip,
                    entry.our_seq,
                    entry.expected_guest_seq,
                    0x11,
                    &[],
                );
                state.tcp.remove(&key);
                return resp;
            }
            Ok(len) => {
                let resp = build_tcp_packet(
                    &entry.client_mac,
                    &entry.client_ip,
                    entry.client_port,
                    entry.remote_port,
                    &entry.remote_ip,
                    entry.our_seq,
                    entry.expected_guest_seq,
                    0x18,
                    &quick_buf[..len],
                );
                entry.our_seq = entry.our_seq.wrapping_add(len as u32);
                return resp;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if !data.is_empty() || ack_flag {
                    return build_tcp_packet(
                        &entry.client_mac,
                        &entry.client_ip,
                        entry.client_port,
                        entry.remote_port,
                        &entry.remote_ip,
                        entry.our_seq,
                        entry.expected_guest_seq,
                        0x10,
                        &[],
                    );
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "TCP read failed");
                let resp = build_tcp_packet(
                    &entry.client_mac,
                    &entry.client_ip,
                    entry.client_port,
                    entry.remote_port,
                    &entry.remote_ip,
                    0,
                    0,
                    0x04,
                    &[],
                );
                state.tcp.remove(&key);
                return resp;
            }
        }
    } else if !data.is_empty() {
        // Can't send but need to ACK guest data
        return build_tcp_packet(
            &entry.client_mac,
            &entry.client_ip,
            entry.client_port,
            entry.remote_port,
            &entry.remote_ip,
            entry.our_seq,
            entry.expected_guest_seq,
            0x10,
            &[],
        );
    }

    None
}

fn handle_tcp_syn(
    state: &mut NatState,
    key: ([u8; 4], u16, u16),
    src_mac: &[u8],
    src_ip: &[u8],
    dst_ip: &[u8],
    src_port: u16,
    dst_port: u16,
    seq: u32,
    syn_options: &[u8],
) -> Option<Vec<u8>> {
    let guest_wscale = parse_tcp_wscale(syn_options).unwrap_or(0).min(14);
    // Translate HOST_IP to localhost
    let (actual_ip, original_ip) = translate_host_ip(dst_ip);

    let dst = SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(
            actual_ip[0],
            actual_ip[1],
            actual_ip[2],
            actual_ip[3],
        )),
        dst_port,
    );

    match TcpStream::connect_timeout(&dst, Duration::from_secs(10)) {
        Ok(stream) => {
            stream.set_nonblocking(true).ok();
            stream.set_nodelay(true).ok();
            // Increase socket buffers for better throughput
            unsafe {
                use std::os::unix::io::AsRawFd;
                let fd = stream.as_raw_fd();
                // Large send buffer to avoid backpressure stalls
                libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    &TCP_SOCKET_SNDBUF as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
                // Large receive buffer to absorb bursts from remote server
                libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_RCVBUF,
                    &TCP_SOCKET_RCVBUF as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }

            let our_seq = 1000u32;
            let expected_guest_seq = seq.wrapping_add(1);

            state.tcp.insert(
                key,
                TcpNatEntry {
                    stream,
                    client_mac: [
                        src_mac[0], src_mac[1], src_mac[2], src_mac[3], src_mac[4], src_mac[5],
                    ],
                    client_ip: [src_ip[0], src_ip[1], src_ip[2], src_ip[3]],
                    client_port: src_port,
                    // Store original_ip so responses go back with the IP guest expects
                    remote_ip: original_ip,
                    remote_port: dst_port,
                    our_seq: our_seq.wrapping_add(1),
                    acked_seq: our_seq, // Guest hasn't ACKed anything yet
                    expected_guest_seq,
                    last_active: Instant::now(),
                    guest_window: 65535,
                    guest_wscale,
                    write_buffer: Vec::new(),
                    write_offset: 0,
                },
            );

            build_tcp_synack(
                src_mac,
                src_ip,
                src_port,
                dst_port,
                &original_ip,
                our_seq,
                expected_guest_seq,
                OUR_WSCALE,
            )
        }
        Err(e) => {
            tracing::debug!(error = %e, "TCP connect failed");
            build_tcp_packet(
                src_mac,
                src_ip,
                src_port,
                dst_port,
                &original_ip,
                0,
                seq.wrapping_add(1),
                0x14,
                &[],
            )
        }
    }
}

fn build_tcp_synack(
    dst_mac: &[u8],
    dst_ip: &[u8],
    dst_port: u16,
    src_port: u16,
    src_ip: &[u8],
    seq: u32,
    ack: u32,
    our_wscale: u8,
) -> Option<Vec<u8>> {
    // TCP options: MSS (4) + WS (4 incl NOP padding) = 8 bytes.
    let mut opts = [0u8; 8];
    // MSS
    opts[0] = 2;
    opts[1] = 4;
    opts[2..4].copy_from_slice(&(MAX_SEGMENT_SIZE as u16).to_be_bytes());
    // NOP + WS
    opts[4] = 1;
    opts[5] = 3;
    opts[6] = 3;
    opts[7] = our_wscale;

    build_tcp_packet_with_options(
        dst_mac,
        dst_ip,
        dst_port,
        src_port,
        src_ip,
        seq,
        ack,
        0x12,
        &opts,
        &[],
    )
}

fn build_tcp_packet(
    dst_mac: &[u8],
    dst_ip: &[u8],
    dst_port: u16,
    src_port: u16,
    src_ip: &[u8],
    seq: u32,
    ack: u32,
    flags: u8,
    data: &[u8],
) -> Option<Vec<u8>> {
    build_tcp_packet_with_options(
        dst_mac,
        dst_ip,
        dst_port,
        src_port,
        src_ip,
        seq,
        ack,
        flags,
        &[],
        data,
    )
}

fn build_tcp_packet_with_options(
    dst_mac: &[u8],
    dst_ip: &[u8],
    dst_port: u16,
    src_port: u16,
    src_ip: &[u8],
    seq: u32,
    ack: u32,
    flags: u8,
    options: &[u8],
    data: &[u8],
) -> Option<Vec<u8>> {
    debug_assert!(options.len() % 4 == 0);
    let tcp_len = 20 + options.len() + data.len();
    let ip = build_ip_header(src_ip, dst_ip, IP_PROTO_TCP, tcp_len, 0);
    let eth = build_eth_header(dst_mac, &GATEWAY_MAC, ETHERTYPE_IPV4);

    let mut response = Vec::with_capacity(14 + 20 + tcp_len);
    response.extend_from_slice(&eth);
    response.extend_from_slice(&ip);

    response.extend_from_slice(&src_port.to_be_bytes());
    response.extend_from_slice(&dst_port.to_be_bytes());
    response.extend_from_slice(&seq.to_be_bytes());
    response.extend_from_slice(&ack.to_be_bytes());

    let doff_words = ((20 + options.len()) / 4) as u8;
    response.push(doff_words << 4);
    response.push(flags);
    response.extend_from_slice(&(u16::MAX).to_be_bytes());
    response.extend_from_slice(&[0, 0]); // checksum placeholder
    response.extend_from_slice(&[0, 0]); // urgent pointer
    response.extend_from_slice(options);
    response.extend_from_slice(data);

    let tcp_start = 14 + 20;
    let tcp_end = tcp_start + tcp_len;
    let cksum = tcp_udp_checksum(src_ip, dst_ip, IP_PROTO_TCP, &response[tcp_start..tcp_end]);
    response[tcp_start + 16..tcp_start + 18].copy_from_slice(&cksum.to_be_bytes());
    tracing::trace!(
        seq,
        ack,
        flags,
        data_len = data.len(),
        opt_len = options.len(),
        "TCP tx"
    );
    Some(response)
}

fn parse_tcp_wscale(options: &[u8]) -> Option<u8> {
    let mut i = 0usize;
    while i < options.len() {
        let kind = options[i];
        match kind {
            0 => break, // EOL
            1 => {
                i += 1; // NOP
                continue;
            }
            _ => {
                if i + 1 >= options.len() {
                    break;
                }
                let len = options[i + 1] as usize;
                if len < 2 || i + len > options.len() {
                    break;
                }
                if kind == 3 && len == 3 {
                    return Some(options[i + 2]);
                }
                i += len;
            }
        }
    }
    None
}

/// Poll NAT sockets for incoming data.
pub fn poll_nat_sockets(state: &mut NatState, responses: &mut Vec<Vec<u8>>) {
    responses.clear();

    // Poll UDP
    for (key, entry) in state.udp.iter_mut() {
        while let Ok(len) = entry.socket.recv(&mut state.udp_rx_buf) {
            if let Some(resp) = build_udp_response(
                &entry.client_mac,
                &entry.client_ip,
                entry.client_port,
                key.1,
                &key.0,
                &state.udp_rx_buf[..len],
            ) {
                responses.push(resp);
            }
        }
    }

    // Poll TCP - batch reads for better throughput
    state.tcp_keys_scratch.clear();
    state.tcp_keys_scratch.extend(state.tcp.keys().cloned());

    for key in state.tcp_keys_scratch.iter().cloned() {
        // First, try to flush any pending write buffer
        if let Some(entry) = state.tcp.get_mut(&key) {
            if entry.write_offset < entry.write_buffer.len() {
                match entry
                    .stream
                    .write(&entry.write_buffer[entry.write_offset..])
                {
                    Ok(0) => {
                        // Connection closed
                        if let Some(resp) = build_tcp_packet(
                            &entry.client_mac,
                            &entry.client_ip,
                            entry.client_port,
                            entry.remote_port,
                            &entry.remote_ip,
                            0,
                            0,
                            0x04,
                            &[],
                        ) {
                            responses.push(resp);
                        }
                        state.tcp.remove(&key);
                        continue;
                    }
                    Ok(n) => {
                        entry.write_offset = entry.write_offset.saturating_add(n);
                        if entry.write_offset > 64 * 1024
                            && entry.write_offset >= entry.write_buffer.len() / 2
                        {
                            compact_write_buffer(entry);
                        } else if entry.write_offset >= entry.write_buffer.len() {
                            entry.write_buffer.clear();
                            entry.write_offset = 0;
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => {
                        if let Some(resp) = build_tcp_packet(
                            &entry.client_mac,
                            &entry.client_ip,
                            entry.client_port,
                            entry.remote_port,
                            &entry.remote_ip,
                            0,
                            0,
                            0x04,
                            &[],
                        ) {
                            responses.push(resp);
                        }
                        state.tcp.remove(&key);
                        continue;
                    }
                }
            }
        }

        // Read data from server to send to guest - batch reads for efficiency
        // Read up to TCP_READ_BUFFER_SIZE bytes at once, then split into MSS-sized segments.
        // This dramatically reduces syscall overhead compared to reading MSS bytes at a time.
        'read_loop: loop {
            let Some(entry) = state.tcp.get_mut(&key) else {
                break;
            };

            if !entry.can_send() {
                break;
            }

            match entry.stream.read(&mut state.tcp_rx_buf) {
                Ok(0) => {
                    // Connection closed
                    if let Some(resp) = build_tcp_packet(
                        &entry.client_mac,
                        &entry.client_ip,
                        entry.client_port,
                        entry.remote_port,
                        &entry.remote_ip,
                        entry.our_seq,
                        entry.expected_guest_seq,
                        0x11,
                        &[],
                    ) {
                        responses.push(resp);
                    }
                    state.tcp.remove(&key);
                    break 'read_loop;
                }
                Ok(total_len) => {
                    // Split into MSS-sized segments for the guest
                    let mut offset = 0;
                    while offset < total_len {
                        let chunk_len = (total_len - offset).min(MAX_SEGMENT_SIZE);
                        let chunk = &state.tcp_rx_buf[offset..offset + chunk_len];

                        if let Some(e) = state.tcp.get(&key) {
                            let seq = e.our_seq.wrapping_add(offset as u32);
                            if let Some(resp) = build_tcp_packet(
                                &e.client_mac,
                                &e.client_ip,
                                e.client_port,
                                e.remote_port,
                                &e.remote_ip,
                                seq,
                                e.expected_guest_seq,
                                0x18,
                                chunk,
                            ) {
                                responses.push(resp);
                            }
                        }
                        offset += chunk_len;
                    }
                    if let Some(e) = state.tcp.get_mut(&key) {
                        e.our_seq = e.our_seq.wrapping_add(total_len as u32);
                    }
                    // If we read less than buffer size, socket is likely drained
                    if total_len < TCP_READ_BUFFER_SIZE / 2 {
                        break 'read_loop;
                    }
                    // Continue reading if there might be more data
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    break 'read_loop;
                }
                Err(_) => {
                    // Error - send RST
                    if let Some(entry) = state.tcp.get(&key)
                        && let Some(resp) = build_tcp_packet(
                            &entry.client_mac,
                            &entry.client_ip,
                            entry.client_port,
                            entry.remote_port,
                            &entry.remote_ip,
                            0,
                            0,
                            0x04,
                            &[],
                        )
                    {
                        responses.push(resp);
                    }
                    state.tcp.remove(&key);
                    break 'read_loop;
                }
            }
        }
    }

    // Cleanup stale connections
    let now = Instant::now();
    state
        .udp
        .retain(|_, e| now.duration_since(e.last_active) < Duration::from_secs(60));
    state
        .tcp
        .retain(|_, e| now.duration_since(e.last_active) < Duration::from_secs(300));
}

#[inline]
fn compact_write_buffer(entry: &mut TcpNatEntry) {
    if entry.write_offset == 0 {
        return;
    }
    if entry.write_offset >= entry.write_buffer.len() {
        entry.write_buffer.clear();
        entry.write_offset = 0;
        return;
    }
    let remaining = entry.write_buffer.len() - entry.write_offset;
    entry.write_buffer.copy_within(entry.write_offset.., 0);
    entry.write_buffer.truncate(remaining);
    entry.write_offset = 0;
}
