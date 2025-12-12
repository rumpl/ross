//! Ethernet frame utilities.

pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IPV4: u16 = 0x0800;

pub const IP_PROTO_ICMP: u8 = 1;
pub const IP_PROTO_TCP: u8 = 6;
pub const IP_PROTO_UDP: u8 = 17;

/// Build an ethernet header.
pub fn build_eth_header(dst: &[u8], src: &[u8], ethertype: u16) -> [u8; 14] {
    let mut hdr = [0u8; 14];
    hdr[0..6].copy_from_slice(dst);
    hdr[6..12].copy_from_slice(src);
    hdr[12..14].copy_from_slice(&ethertype.to_be_bytes());
    hdr
}

/// Calculate IP/ICMP/TCP/UDP checksum.
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i < data.len() {
        let word = if i + 1 < data.len() {
            u16::from_be_bytes([data[i], data[i + 1]])
        } else {
            u16::from_be_bytes([data[i], 0])
        };
        sum += word as u32;
        i += 2;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Calculate TCP/UDP checksum with pseudo-header.
pub fn tcp_udp_checksum(src_ip: &[u8], dst_ip: &[u8], proto: u8, data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    
    // Pseudo-header
    sum += u16::from_be_bytes([src_ip[0], src_ip[1]]) as u32;
    sum += u16::from_be_bytes([src_ip[2], src_ip[3]]) as u32;
    sum += u16::from_be_bytes([dst_ip[0], dst_ip[1]]) as u32;
    sum += u16::from_be_bytes([dst_ip[2], dst_ip[3]]) as u32;
    sum += proto as u32;
    sum += data.len() as u32;
    
    // Data
    let mut i = 0;
    while i < data.len() {
        let word = if i + 1 < data.len() {
            u16::from_be_bytes([data[i], data[i + 1]])
        } else {
            u16::from_be_bytes([data[i], 0])
        };
        sum += word as u32;
        i += 2;
    }
    
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Build an IPv4 header.
pub fn build_ip_header(
    src: &[u8],
    dst: &[u8],
    proto: u8,
    payload_len: usize,
    id: u16,
) -> [u8; 20] {
    let total_len = (20 + payload_len) as u16;
    let mut hdr = [0u8; 20];
    hdr[0] = 0x45; // version + IHL
    hdr[1] = 0;    // DSCP + ECN
    hdr[2..4].copy_from_slice(&total_len.to_be_bytes());
    hdr[4..6].copy_from_slice(&id.to_be_bytes());
    hdr[6..8].copy_from_slice(&[0x40, 0]); // Don't fragment
    hdr[8] = 64;   // TTL
    hdr[9] = proto;
    // Checksum at [10..12] - computed below
    hdr[12..16].copy_from_slice(src);
    hdr[16..20].copy_from_slice(dst);
    
    let cksum = checksum(&hdr);
    hdr[10..12].copy_from_slice(&cksum.to_be_bytes());
    hdr
}
