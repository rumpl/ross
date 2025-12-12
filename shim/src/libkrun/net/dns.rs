//! DNS forwarding.

use super::eth::{build_eth_header, build_ip_header, tcp_udp_checksum, ETHERTYPE_IPV4, IP_PROTO_UDP};
use super::{GATEWAY_IP, GATEWAY_MAC};
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

/// Handle DNS query by forwarding to upstream.
pub fn handle_dns(
    query: &[u8],
    client_mac: &[u8],
    client_ip: &[u8],
    client_port: u16,
) -> Option<Vec<u8>> {
    if query.len() < 12 {
        return None;
    }

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
