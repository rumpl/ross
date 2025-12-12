//! NAT for TCP and UDP connections.

use super::eth::{
    build_eth_header, build_ip_header, checksum, tcp_udp_checksum, ETHERTYPE_IPV4, IP_PROTO_ICMP,
    IP_PROTO_TCP, IP_PROTO_UDP,
};
use super::{GATEWAY_MAC, HOST_IP};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

const MAX_SEGMENT_SIZE: usize = 1400;
const TCP_WINDOW: u32 = 65535;

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
    last_active: Instant,
}

impl TcpNatEntry {
    fn can_send(&self) -> bool {
        // Simple flow control: only send if we haven't sent too much unacked data
        let unacked = self.our_seq.wrapping_sub(self.acked_seq);
        unacked < TCP_WINDOW
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
}

impl NatState {
    pub fn new() -> Self {
        Self {
            tcp: HashMap::new(),
            udp: HashMap::new(),
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

fn build_icmp_reply(dst_mac: &[u8], dst_ip: &[u8], src_ip: &[u8], request: &[u8]) -> Option<Vec<u8>> {
    let mut icmp = request.to_vec();
    icmp[0] = 0;
    icmp[2..4].copy_from_slice(&[0, 0]);
    let cksum = checksum(&icmp);
    icmp[2..4].copy_from_slice(&cksum.to_be_bytes());

    let ip = build_ip_header(src_ip, dst_ip, IP_PROTO_ICMP, icmp.len(), 0);
    let eth = build_eth_header(dst_mac, &GATEWAY_MAC, ETHERTYPE_IPV4);

    let mut response = Vec::with_capacity(14 + 20 + icmp.len());
    response.extend_from_slice(&eth);
    response.extend_from_slice(&ip);
    response.extend_from_slice(&icmp);
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
            IpAddr::V4(Ipv4Addr::new(actual_ip[0], actual_ip[1], actual_ip[2], actual_ip[3])),
            dst_port,
        );
        socket.connect(dst).ok();
        UdpNatEntry {
            socket,
            client_mac: [src_mac[0], src_mac[1], src_mac[2], src_mac[3], src_mac[4], src_mac[5]],
            client_ip: [src_ip[0], src_ip[1], src_ip[2], src_ip[3]],
            client_port: src_port,
            last_active: Instant::now(),
        }
    });

    entry.last_active = Instant::now();
    let _ = entry.socket.send(data);

    let mut buf = [0u8; 65535];
    if let Ok(len) = entry.socket.recv(&mut buf) {
        // Use original_ip in response so guest sees the IP it connected to
        return build_udp_response(
            &entry.client_mac, &entry.client_ip, entry.client_port,
            dst_port, &original_ip, &buf[..len],
        );
    }
    None
}

fn build_udp_response(
    dst_mac: &[u8], dst_ip: &[u8], dst_port: u16,
    src_port: u16, src_ip: &[u8], data: &[u8],
) -> Option<Vec<u8>> {
    let udp_len = 8 + data.len();
    let mut udp = Vec::with_capacity(udp_len);
    udp.extend_from_slice(&src_port.to_be_bytes());
    udp.extend_from_slice(&dst_port.to_be_bytes());
    udp.extend_from_slice(&(udp_len as u16).to_be_bytes());
    udp.extend_from_slice(&[0, 0]);
    udp.extend_from_slice(data);
    let cksum = tcp_udp_checksum(src_ip, dst_ip, IP_PROTO_UDP, &udp);
    udp[6..8].copy_from_slice(&cksum.to_be_bytes());

    let ip = build_ip_header(src_ip, dst_ip, IP_PROTO_UDP, udp_len, 0);
    let eth = build_eth_header(dst_mac, &GATEWAY_MAC, ETHERTYPE_IPV4);

    let mut response = Vec::with_capacity(14 + 20 + udp_len);
    response.extend_from_slice(&eth);
    response.extend_from_slice(&ip);
    response.extend_from_slice(&udp);
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

    let syn = flags & 0x02 != 0;
    let ack_flag = flags & 0x10 != 0;
    let fin = flags & 0x01 != 0;
    let rst = flags & 0x04 != 0;

    let data = if data_offset < payload.len() { &payload[data_offset..] } else { &[] };
    let key = ([dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3]], dst_port, src_port);

    tracing::trace!(
        src_port, dst_port, seq, ack, syn, ack_flag, fin, rst,
        data_len = data.len(),
        "TCP rx"
    );

    if rst {
        state.tcp.remove(&key);
        return None;
    }

    // SYN - new connection
    if syn && !ack_flag {
        return handle_tcp_syn(state, key, src_mac, src_ip, dst_ip, src_port, dst_port, seq);
    }

    let entry = state.tcp.get_mut(&key)?;
    entry.last_active = Instant::now();

    // Update acked_seq from guest's ACK
    if ack_flag && ack > entry.acked_seq {
        entry.acked_seq = ack;
    }

    // Handle retransmit
    if seq < entry.expected_guest_seq {
        return build_tcp_packet(
            &entry.client_mac, &entry.client_ip, entry.client_port,
            entry.remote_port, &entry.remote_ip,
            entry.our_seq, entry.expected_guest_seq, 0x10, &[],
        );
    }

    // Out of order
    if seq > entry.expected_guest_seq && !data.is_empty() {
        return build_tcp_packet(
            &entry.client_mac, &entry.client_ip, entry.client_port,
            entry.remote_port, &entry.remote_ip,
            entry.our_seq, entry.expected_guest_seq, 0x10, &[],
        );
    }

    // Process data from guest
    if !data.is_empty() {
        if let Err(e) = entry.stream.write_all(data) {
            tracing::debug!(error = %e, "TCP write failed");
            let resp = build_tcp_packet(
                &entry.client_mac, &entry.client_ip, entry.client_port,
                entry.remote_port, &entry.remote_ip,
                0, 0, 0x04, &[],
            );
            state.tcp.remove(&key);
            return resp;
        }
        entry.expected_guest_seq = entry.expected_guest_seq.wrapping_add(data.len() as u32);
    }

    // FIN
    if fin {
        entry.expected_guest_seq = entry.expected_guest_seq.wrapping_add(1);
        let resp = build_tcp_packet(
            &entry.client_mac, &entry.client_ip, entry.client_port,
            entry.remote_port, &entry.remote_ip,
            entry.our_seq, entry.expected_guest_seq, 0x11, &[],
        );
        state.tcp.remove(&key);
        return resp;
    }

    // Try to send data to guest if we have window space
    if entry.can_send() {
        let mut buf = [0u8; MAX_SEGMENT_SIZE];
        match entry.stream.read(&mut buf) {
            Ok(0) => {
                let resp = build_tcp_packet(
                    &entry.client_mac, &entry.client_ip, entry.client_port,
                    entry.remote_port, &entry.remote_ip,
                    entry.our_seq, entry.expected_guest_seq, 0x11, &[],
                );
                state.tcp.remove(&key);
                return resp;
            }
            Ok(len) => {
                let resp = build_tcp_packet(
                    &entry.client_mac, &entry.client_ip, entry.client_port,
                    entry.remote_port, &entry.remote_ip,
                    entry.our_seq, entry.expected_guest_seq, 0x18, &buf[..len],
                );
                entry.our_seq = entry.our_seq.wrapping_add(len as u32);
                return resp;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if !data.is_empty() || ack_flag {
                    return build_tcp_packet(
                        &entry.client_mac, &entry.client_ip, entry.client_port,
                        entry.remote_port, &entry.remote_ip,
                        entry.our_seq, entry.expected_guest_seq, 0x10, &[],
                    );
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "TCP read failed");
                let resp = build_tcp_packet(
                    &entry.client_mac, &entry.client_ip, entry.client_port,
                    entry.remote_port, &entry.remote_ip,
                    0, 0, 0x04, &[],
                );
                state.tcp.remove(&key);
                return resp;
            }
        }
    } else if !data.is_empty() {
        // Can't send but need to ACK guest data
        return build_tcp_packet(
            &entry.client_mac, &entry.client_ip, entry.client_port,
            entry.remote_port, &entry.remote_ip,
            entry.our_seq, entry.expected_guest_seq, 0x10, &[],
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
) -> Option<Vec<u8>> {
    // Translate HOST_IP to localhost
    let (actual_ip, original_ip) = translate_host_ip(dst_ip);

    let dst = SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(actual_ip[0], actual_ip[1], actual_ip[2], actual_ip[3])),
        dst_port,
    );

    match TcpStream::connect_timeout(&dst, Duration::from_secs(10)) {
        Ok(stream) => {
            stream.set_nonblocking(true).ok();
            stream.set_nodelay(true).ok();

            let our_seq = 1000u32;
            let expected_guest_seq = seq.wrapping_add(1);

            state.tcp.insert(key, TcpNatEntry {
                stream,
                client_mac: [src_mac[0], src_mac[1], src_mac[2], src_mac[3], src_mac[4], src_mac[5]],
                client_ip: [src_ip[0], src_ip[1], src_ip[2], src_ip[3]],
                client_port: src_port,
                // Store original_ip so responses go back with the IP guest expects
                remote_ip: original_ip,
                remote_port: dst_port,
                our_seq: our_seq.wrapping_add(1),
                acked_seq: our_seq, // Guest hasn't ACKed anything yet
                expected_guest_seq,
                last_active: Instant::now(),
            });

            build_tcp_packet(src_mac, src_ip, src_port, dst_port, &original_ip, our_seq, expected_guest_seq, 0x12, &[])
        }
        Err(e) => {
            tracing::debug!(error = %e, "TCP connect failed");
            build_tcp_packet(src_mac, src_ip, src_port, dst_port, &original_ip, 0, seq.wrapping_add(1), 0x14, &[])
        }
    }
}

fn build_tcp_packet(
    dst_mac: &[u8], dst_ip: &[u8], dst_port: u16,
    src_port: u16, src_ip: &[u8],
    seq: u32, ack: u32, flags: u8, data: &[u8],
) -> Option<Vec<u8>> {
    let tcp_len = 20 + data.len();
    let mut tcp = vec![0u8; tcp_len];

    tcp[0..2].copy_from_slice(&src_port.to_be_bytes());
    tcp[2..4].copy_from_slice(&dst_port.to_be_bytes());
    tcp[4..8].copy_from_slice(&seq.to_be_bytes());
    tcp[8..12].copy_from_slice(&ack.to_be_bytes());
    tcp[12] = 0x50;
    tcp[13] = flags;
    tcp[14..16].copy_from_slice(&(TCP_WINDOW as u16).to_be_bytes());
    tcp[20..].copy_from_slice(data);

    let cksum = tcp_udp_checksum(src_ip, dst_ip, IP_PROTO_TCP, &tcp);
    tcp[16..18].copy_from_slice(&cksum.to_be_bytes());

    let ip = build_ip_header(src_ip, dst_ip, IP_PROTO_TCP, tcp_len, 0);
    let eth = build_eth_header(dst_mac, &GATEWAY_MAC, ETHERTYPE_IPV4);

    let mut response = Vec::with_capacity(14 + 20 + tcp_len);
    response.extend_from_slice(&eth);
    response.extend_from_slice(&ip);
    response.extend_from_slice(&tcp);

    tracing::trace!(seq, ack, flags = format!("0x{:02x}", flags), data_len = data.len(), "TCP tx");
    Some(response)
}

/// Poll NAT sockets for incoming data.
pub fn poll_nat_sockets(state: &mut NatState) -> Vec<Vec<u8>> {
    let mut responses = Vec::new();

    // Poll UDP
    let udp_keys: Vec<_> = state.udp.keys().cloned().collect();
    for key in udp_keys {
        if let Some(entry) = state.udp.get_mut(&key) {
            let mut buf = [0u8; 65535];
            while let Ok(len) = entry.socket.recv(&mut buf) {
                if let Some(resp) = build_udp_response(
                    &entry.client_mac, &entry.client_ip, entry.client_port,
                    key.1, &key.0, &buf[..len],
                ) {
                    responses.push(resp);
                }
            }
        }
    }

    // Poll TCP - only send one packet per connection per poll to avoid flooding
    let tcp_keys: Vec<_> = state.tcp.keys().cloned().collect();
    for key in tcp_keys {
        if let Some(entry) = state.tcp.get_mut(&key) {
            if !entry.can_send() {
                continue;
            }

            let mut buf = [0u8; MAX_SEGMENT_SIZE];
            match entry.stream.read(&mut buf) {
                Ok(0) => {
                    if let Some(resp) = build_tcp_packet(
                        &entry.client_mac, &entry.client_ip, entry.client_port,
                        entry.remote_port, &entry.remote_ip,
                        entry.our_seq, entry.expected_guest_seq, 0x11, &[],
                    ) {
                        responses.push(resp);
                    }
                    state.tcp.remove(&key);
                }
                Ok(len) => {
                    if let Some(resp) = build_tcp_packet(
                        &entry.client_mac, &entry.client_ip, entry.client_port,
                        entry.remote_port, &entry.remote_ip,
                        entry.our_seq, entry.expected_guest_seq, 0x18, &buf[..len],
                    ) {
                        responses.push(resp);
                    }
                    if let Some(e) = state.tcp.get_mut(&key) {
                        e.our_seq = e.our_seq.wrapping_add(len as u32);
                    }
                }
                Err(_) => {}
            }
        }
    }

    // Cleanup
    let now = Instant::now();
    state.udp.retain(|_, e| now.duration_since(e.last_active) < Duration::from_secs(60));
    state.tcp.retain(|_, e| now.duration_since(e.last_active) < Duration::from_secs(300));

    responses
}
