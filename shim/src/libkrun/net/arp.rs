//! ARP handling.

use super::eth::{build_eth_header, ETHERTYPE_ARP};
use super::{GATEWAY_IP, GATEWAY_MAC, HOST_IP};

/// Handle ARP request and return response if applicable.
pub fn handle_arp(payload: &[u8], src_mac: &[u8]) -> Option<Vec<u8>> {
    if payload.len() < 28 {
        return None;
    }

    let operation = u16::from_be_bytes([payload[6], payload[7]]);
    if operation != 1 {
        return None; // Only handle requests
    }

    let target_ip = &payload[24..28];
    
    // Respond for gateway IP and host IP (ross.host.internal)
    let is_gateway = target_ip == GATEWAY_IP;
    let is_host = target_ip == HOST_IP;
    
    if !is_gateway && !is_host {
        return None;
    }

    if is_gateway {
        tracing::debug!("ARP request for gateway");
    } else {
        tracing::debug!("ARP request for host (ross.host.internal)");
    }

    let mut response = Vec::with_capacity(14 + 28);
    response.extend_from_slice(&build_eth_header(src_mac, &GATEWAY_MAC, ETHERTYPE_ARP));

    // ARP reply - use gateway MAC for both gateway and host IP
    let mut arp = [0u8; 28];
    arp[0..2].copy_from_slice(&[0, 1]);       // hardware type: ethernet
    arp[2..4].copy_from_slice(&[0x08, 0]);    // protocol type: IPv4
    arp[4] = 6;                                // hardware size
    arp[5] = 4;                                // protocol size
    arp[6..8].copy_from_slice(&[0, 2]);       // operation: reply
    arp[8..14].copy_from_slice(&GATEWAY_MAC); // sender MAC
    arp[14..18].copy_from_slice(target_ip);   // sender IP (the IP being requested)
    arp[18..24].copy_from_slice(src_mac);     // target MAC
    arp[24..28].copy_from_slice(&payload[14..18]); // target IP

    response.extend_from_slice(&arp);
    Some(response)
}
