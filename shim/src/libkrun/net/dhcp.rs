//! DHCP server.

use super::eth::{ETHERTYPE_IPV4, IP_PROTO_UDP, build_eth_header, build_ip_header};
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

    let mut dhcp = [0u8; 300];
    let dhcp_len = build_dhcp_response(payload, response_type, &mut dhcp);

    let udp_len = 8 + dhcp_len;
    let ip = build_ip_header(&GATEWAY_IP, &[255, 255, 255, 255], IP_PROTO_UDP, udp_len, 0);
    let eth = build_eth_header(&[0xff; 6], &GATEWAY_MAC, ETHERTYPE_IPV4);

    let mut response = Vec::with_capacity(14 + 20 + udp_len);
    response.extend_from_slice(&eth);
    response.extend_from_slice(&ip);
    // UDP header (checksum optional; left zero).
    response.extend_from_slice(&67u16.to_be_bytes());
    response.extend_from_slice(&68u16.to_be_bytes());
    response.extend_from_slice(&(udp_len as u16).to_be_bytes());
    response.extend_from_slice(&[0, 0]);
    response.extend_from_slice(&dhcp[..dhcp_len]);

    tracing::info!(
        response = if response_type == 2 { "OFFER" } else { "ACK" },
        ip = format!(
            "{}.{}.{}.{}",
            GUEST_IP[0], GUEST_IP[1], GUEST_IP[2], GUEST_IP[3]
        ),
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

fn build_dhcp_response(request: &[u8], msg_type: u8, out: &mut [u8; 300]) -> usize {
    out.fill(0);

    out[0] = 2; // BOOTREPLY
    out[1] = 1; // Ethernet
    out[2] = 6; // MAC length
    out[4..8].copy_from_slice(&request[4..8]); // Transaction ID
    out[10..12].copy_from_slice(&[0x80, 0]); // Broadcast flag
    out[16..20].copy_from_slice(&GUEST_IP); // Your IP
    out[20..24].copy_from_slice(&GATEWAY_IP); // Server IP
    out[28..34].copy_from_slice(&request[28..34]); // Client MAC

    // Magic cookie
    out[236..240].copy_from_slice(&[99, 130, 83, 99]);

    // Options
    let mut i = 240;

    // Message type
    out[i] = 53;
    out[i + 1] = 1;
    out[i + 2] = msg_type;
    i += 3;

    // Server identifier
    out[i] = 54;
    out[i + 1] = 4;
    out[i + 2..i + 6].copy_from_slice(&GATEWAY_IP);
    i += 6;

    // Lease time (24h)
    out[i] = 51;
    out[i + 1] = 4;
    out[i + 2..i + 6].copy_from_slice(&86400u32.to_be_bytes());
    i += 6;

    // Subnet mask
    out[i] = 1;
    out[i + 1] = 4;
    out[i + 2..i + 6].copy_from_slice(&SUBNET_MASK);
    i += 6;

    // Router
    out[i] = 3;
    out[i + 1] = 4;
    out[i + 2..i + 6].copy_from_slice(&GATEWAY_IP);
    i += 6;

    // DNS
    out[i] = 6;
    out[i + 1] = 4;
    out[i + 2..i + 6].copy_from_slice(&GATEWAY_IP);
    i += 6;

    // End
    out[i] = 255;
    i += 1;
    i
}
