//! DHCP server.

use super::eth::{build_eth_header, build_ip_header, ETHERTYPE_IPV4, IP_PROTO_UDP};
use super::{GATEWAY_IP, GATEWAY_MAC, GUEST_IP, SUBNET_MASK};

/// Handle DHCP request and return response.
pub fn handle_dhcp(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() < 240 {
        return None;
    }

    if payload[0] != 1 {
        return None; // Only BOOTREQUEST
    }

    // Find message type in options
    let msg_type = find_dhcp_option(&payload[240..], 53)?;
    let response_type = match msg_type {
        1 => 2, // DISCOVER -> OFFER
        3 => 5, // REQUEST -> ACK
        _ => return None,
    };

    tracing::debug!(msg_type = msg_type, "DHCP request");

    let dhcp = build_dhcp_response(payload, response_type);
    let udp = build_udp_packet(67, 68, &dhcp);
    let ip = build_ip_header(&GATEWAY_IP, &[255, 255, 255, 255], IP_PROTO_UDP, udp.len(), 0);
    let eth = build_eth_header(&[0xff; 6], &GATEWAY_MAC, ETHERTYPE_IPV4);

    let mut response = Vec::with_capacity(14 + 20 + udp.len());
    response.extend_from_slice(&eth);
    response.extend_from_slice(&ip);
    response.extend_from_slice(&udp);

    tracing::info!(
        response = if response_type == 2 { "OFFER" } else { "ACK" },
        ip = format!("{}.{}.{}.{}", GUEST_IP[0], GUEST_IP[1], GUEST_IP[2], GUEST_IP[3]),
        "DHCP response"
    );

    Some(response)
}

fn find_dhcp_option(options: &[u8], opt_code: u8) -> Option<u8> {
    let mut i = 0;
    while i < options.len() {
        let code = options[i];
        if code == 255 {
            break;
        }
        if code == 0 {
            i += 1;
            continue;
        }
        if i + 1 >= options.len() {
            break;
        }
        let len = options[i + 1] as usize;
        if code == opt_code && len >= 1 && i + 2 < options.len() {
            return Some(options[i + 2]);
        }
        i += 2 + len;
    }
    None
}

fn build_dhcp_response(request: &[u8], msg_type: u8) -> Vec<u8> {
    let mut dhcp = vec![0u8; 300];

    dhcp[0] = 2; // BOOTREPLY
    dhcp[1] = 1; // Ethernet
    dhcp[2] = 6; // MAC length
    dhcp[4..8].copy_from_slice(&request[4..8]); // Transaction ID
    dhcp[10..12].copy_from_slice(&[0x80, 0]);   // Broadcast flag
    dhcp[16..20].copy_from_slice(&GUEST_IP);    // Your IP
    dhcp[20..24].copy_from_slice(&GATEWAY_IP);  // Server IP
    dhcp[28..34].copy_from_slice(&request[28..34]); // Client MAC

    // Magic cookie
    dhcp[236..240].copy_from_slice(&[99, 130, 83, 99]);

    // Options
    let mut i = 240;
    
    // Message type
    dhcp[i] = 53; dhcp[i+1] = 1; dhcp[i+2] = msg_type;
    i += 3;

    // Server identifier
    dhcp[i] = 54; dhcp[i+1] = 4;
    dhcp[i+2..i+6].copy_from_slice(&GATEWAY_IP);
    i += 6;

    // Lease time (24h)
    dhcp[i] = 51; dhcp[i+1] = 4;
    dhcp[i+2..i+6].copy_from_slice(&86400u32.to_be_bytes());
    i += 6;

    // Subnet mask
    dhcp[i] = 1; dhcp[i+1] = 4;
    dhcp[i+2..i+6].copy_from_slice(&SUBNET_MASK);
    i += 6;

    // Router
    dhcp[i] = 3; dhcp[i+1] = 4;
    dhcp[i+2..i+6].copy_from_slice(&GATEWAY_IP);
    i += 6;

    // DNS
    dhcp[i] = 6; dhcp[i+1] = 4;
    dhcp[i+2..i+6].copy_from_slice(&GATEWAY_IP);
    i += 6;

    // End
    dhcp[i] = 255;
    i += 1;

    dhcp.truncate(i);
    dhcp
}

fn build_udp_packet(src_port: u16, dst_port: u16, data: &[u8]) -> Vec<u8> {
    let len = (8 + data.len()) as u16;
    let mut udp = Vec::with_capacity(8 + data.len());
    udp.extend_from_slice(&src_port.to_be_bytes());
    udp.extend_from_slice(&dst_port.to_be_bytes());
    udp.extend_from_slice(&len.to_be_bytes());
    udp.extend_from_slice(&[0, 0]); // Checksum (optional)
    udp.extend_from_slice(data);
    udp
}
