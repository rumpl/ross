//! DNS forwarding with special handling for ross.host.internal.

use super::eth::{build_eth_header, build_ip_header, tcp_udp_checksum, ETHERTYPE_IPV4, IP_PROTO_UDP};
use super::{GATEWAY_IP, GATEWAY_MAC, HOST_IP};
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

const ROSS_HOST_INTERNAL: &str = "ross.host.internal";
const DEFAULT_DNS_SERVER: &str = "8.8.8.8:53";

/// Persistent UDP socket for forwarding DNS queries.
///
/// Creating/binding sockets per DNS packet is extremely expensive; keeping a single
/// connected socket avoids repeated syscalls and kernel allocations.
pub struct DnsForwarder {
    socket: UdpSocket,
}

impl DnsForwarder {
    pub fn new() -> Option<Self> {
        let dns_server: SocketAddr = DEFAULT_DNS_SERVER.parse().ok()?;
        let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
        // A connected UDP socket avoids specifying the destination on every send.
        socket.connect(dns_server).ok()?;
        socket.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        Some(Self { socket })
    }

    #[inline]
    fn send_query(&self, query: &[u8]) -> bool {
        self.socket.send(query).is_ok()
    }

    #[inline]
    fn recv_response<'a>(&self, buf: &'a mut [u8]) -> Option<&'a [u8]> {
        let len = self.socket.recv(buf).ok()?;
        Some(&buf[..len])
    }
}

/// Handle DNS query by forwarding to upstream or resolving special hostnames.
pub fn handle_dns(
    query: &[u8],
    client_mac: &[u8],
    client_ip: &[u8],
    client_port: u16,
    forwarder: &mut Option<DnsForwarder>,
) -> Option<Vec<u8>> {
    if query.len() < 12 {
        return None;
    }

    // Check if this is a query for ross.host.internal
    if is_query_for_ross_host_internal(query) {
        tracing::debug!(name = ROSS_HOST_INTERNAL, "Resolving special hostname to host IP");
        if let Some(response) = build_dns_response(query, &HOST_IP) {
            return build_udp_response(client_mac, client_ip, client_port, 53, &response);
        }
    }

    // Forward to upstream DNS
    if forwarder.is_none() {
        *forwarder = DnsForwarder::new();
    }

    let fwd = forwarder.as_ref()?;
    if !fwd.send_query(query) {
        return None;
    }

    let mut buf = [0u8; 512];
    let response = fwd.recv_response(&mut buf)?;

    tracing::debug!(len = response.len(), "DNS response");

    build_udp_response(client_mac, client_ip, client_port, 53, response)
}

#[inline]
fn eq_ascii_case_insensitive(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(&x, &y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

/// Fast path: check if the first DNS question name matches `ross.host.internal`
/// without allocating.
fn is_query_for_ross_host_internal(query: &[u8]) -> bool {
    const LABELS: [&[u8]; 3] = [b"ross", b"host", b"internal"];

    // DNS header is 12 bytes, question section starts after.
    let mut pos = 12usize;
    let mut label_idx = 0usize;

    while pos < query.len() {
        let len = query[pos] as usize;
        pos += 1;

        if len == 0 {
            // End of QNAME. Must have matched exactly 3 labels.
            return label_idx == LABELS.len();
        }

        // Compression pointers in QNAME aren't expected in queries we originate; bail out.
        if len & 0b1100_0000 != 0 {
            return false;
        }

        if pos + len > query.len() {
            return false;
        }

        if label_idx >= LABELS.len() {
            return false;
        }

        if !eq_ascii_case_insensitive(&query[pos..pos + len], LABELS[label_idx]) {
            return false;
        }

        pos += len;
        label_idx += 1;
    }

    false
}

/// Build a DNS response for an A record query.
fn build_dns_response(query: &[u8], ip: &[u8; 4]) -> Option<Vec<u8>> {
    if query.len() < 12 {
        return None;
    }

    // Find the end of the question section
    let mut pos = 12;
    while pos < query.len() && query[pos] != 0 {
        let len = query[pos] as usize;
        pos += 1 + len;
    }
    // Skip null terminator + QTYPE (2) + QCLASS (2)
    let question_end = pos + 5;
    if question_end > query.len() {
        return None;
    }

    let mut response = Vec::with_capacity(query.len() + 16);

    // Copy transaction ID
    response.extend_from_slice(&query[0..2]);

    // Flags: standard response, no error
    // QR=1 (response), Opcode=0, AA=1 (authoritative), TC=0, RD=1, RA=1, Z=0, RCODE=0
    response.extend_from_slice(&[0x85, 0x80]);

    // QDCOUNT = 1
    response.extend_from_slice(&[0x00, 0x01]);
    // ANCOUNT = 1
    response.extend_from_slice(&[0x00, 0x01]);
    // NSCOUNT = 0
    response.extend_from_slice(&[0x00, 0x00]);
    // ARCOUNT = 0
    response.extend_from_slice(&[0x00, 0x00]);

    // Copy question section
    response.extend_from_slice(&query[12..question_end]);

    // Answer section - use pointer to name in question (0xC00C = offset 12)
    response.extend_from_slice(&[0xC0, 0x0C]);
    // TYPE = A (1)
    response.extend_from_slice(&[0x00, 0x01]);
    // CLASS = IN (1)
    response.extend_from_slice(&[0x00, 0x01]);
    // TTL = 60 seconds
    response.extend_from_slice(&[0x00, 0x00, 0x00, 0x3C]);
    // RDLENGTH = 4
    response.extend_from_slice(&[0x00, 0x04]);
    // RDATA = IP address
    response.extend_from_slice(ip);

    Some(response)
}

fn build_udp_response(
    dst_mac: &[u8],
    dst_ip: &[u8],
    dst_port: u16,
    src_port: u16,
    data: &[u8],
) -> Option<Vec<u8>> {
    let udp_len = 8 + data.len();
    let total_len = 14 + 20 + udp_len;

    let eth = build_eth_header(dst_mac, &GATEWAY_MAC, ETHERTYPE_IPV4);
    let ip = build_ip_header(&GATEWAY_IP, dst_ip, IP_PROTO_UDP, udp_len, 0);

    let mut response = Vec::with_capacity(total_len);
    response.extend_from_slice(&eth);
    response.extend_from_slice(&ip);

    // UDP header (checksum filled after payload copy).
    response.extend_from_slice(&src_port.to_be_bytes());
    response.extend_from_slice(&dst_port.to_be_bytes());
    response.extend_from_slice(&(udp_len as u16).to_be_bytes());
    response.extend_from_slice(&[0, 0]);

    response.extend_from_slice(data);

    // Compute UDP checksum over the UDP segment we just appended.
    let udp_start = 14 + 20;
    let udp_end = udp_start + udp_len;
    let cksum = tcp_udp_checksum(&GATEWAY_IP, dst_ip, IP_PROTO_UDP, &response[udp_start..udp_end]);
    response[udp_start + 6..udp_start + 8].copy_from_slice(&cksum.to_be_bytes());

    Some(response)
}
