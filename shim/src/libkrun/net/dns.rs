//! DNS forwarding with special handling for ross.host.internal.

use super::eth::{build_eth_header, build_ip_header, tcp_udp_checksum, ETHERTYPE_IPV4, IP_PROTO_UDP};
use super::{GATEWAY_IP, GATEWAY_MAC, HOST_IP};
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

const ROSS_HOST_INTERNAL: &str = "ross.host.internal";

/// Handle DNS query by forwarding to upstream or resolving special hostnames.
pub fn handle_dns(
    query: &[u8],
    client_mac: &[u8],
    client_ip: &[u8],
    client_port: u16,
) -> Option<Vec<u8>> {
    if query.len() < 12 {
        return None;
    }

    // Check if this is a query for ross.host.internal
    if let Some(name) = parse_dns_query_name(query)
        && name.eq_ignore_ascii_case(ROSS_HOST_INTERNAL)
    {
        tracing::debug!(name = %name, "Resolving special hostname to host IP");
        if let Some(response) = build_dns_response(query, &HOST_IP) {
            return build_udp_response(client_mac, client_ip, client_port, 53, &response);
        }
    }

    // Forward to upstream DNS
    let dns_server: SocketAddr = "8.8.8.8:53".parse().ok()?;
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    socket.send_to(query, dns_server).ok()?;

    let mut buf = [0u8; 512];
    let (len, _) = socket.recv_from(&mut buf).ok()?;
    let response = &buf[..len];

    tracing::debug!(len = len, "DNS response");

    build_udp_response(client_mac, client_ip, client_port, 53, response)
}

/// Parse the query name from a DNS query packet.
fn parse_dns_query_name(query: &[u8]) -> Option<String> {
    if query.len() < 12 {
        return None;
    }

    // DNS header is 12 bytes, question section starts after
    let mut pos = 12;
    let mut name_parts = Vec::new();

    while pos < query.len() {
        let len = query[pos] as usize;
        if len == 0 {
            break;
        }
        if pos + 1 + len > query.len() {
            return None;
        }
        let label = std::str::from_utf8(&query[pos + 1..pos + 1 + len]).ok()?;
        name_parts.push(label.to_string());
        pos += 1 + len;
    }

    if name_parts.is_empty() {
        return None;
    }

    Some(name_parts.join("."))
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
    let mut udp = Vec::with_capacity(udp_len);
    udp.extend_from_slice(&src_port.to_be_bytes());
    udp.extend_from_slice(&dst_port.to_be_bytes());
    udp.extend_from_slice(&(udp_len as u16).to_be_bytes());
    udp.extend_from_slice(&[0, 0]); // Checksum placeholder
    udp.extend_from_slice(data);

    let cksum = tcp_udp_checksum(&GATEWAY_IP, dst_ip, IP_PROTO_UDP, &udp);
    udp[6..8].copy_from_slice(&cksum.to_be_bytes());

    let ip = build_ip_header(&GATEWAY_IP, dst_ip, IP_PROTO_UDP, udp_len, 0);
    let eth = build_eth_header(dst_mac, &GATEWAY_MAC, ETHERTYPE_IPV4);

    let mut response = Vec::with_capacity(14 + 20 + udp_len);
    response.extend_from_slice(&eth);
    response.extend_from_slice(&ip);
    response.extend_from_slice(&udp);

    Some(response)
}
