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
    finalize_checksum(sum_be_words(data))
}

/// Calculate TCP/UDP checksum with pseudo-header.
pub fn tcp_udp_checksum(src_ip: &[u8], dst_ip: &[u8], proto: u8, data: &[u8]) -> u16 {
    // Pseudo-header
    let mut sum = 0u64;
    sum += u16::from_be_bytes([src_ip[0], src_ip[1]]) as u64;
    sum += u16::from_be_bytes([src_ip[2], src_ip[3]]) as u64;
    sum += u16::from_be_bytes([dst_ip[0], dst_ip[1]]) as u64;
    sum += u16::from_be_bytes([dst_ip[2], dst_ip[3]]) as u64;
    sum += proto as u64;
    sum += data.len() as u64;
    sum += sum_be_words(data);
    finalize_checksum(sum)
}

#[inline]
fn finalize_checksum(mut sum: u64) -> u16 {
    // Fold carries into 16 bits.
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Sum 16-bit big-endian words in `data` into a 64-bit accumulator.
///
/// Optimized for little-endian hosts (macOS/aarch64): processes 8 bytes at a time.
#[inline]
fn sum_be_words(data: &[u8]) -> u64 {
    #[cfg(target_endian = "little")]
    {
        let mut sum = 0u64;
        let mut i = 0usize;

        // 8-byte chunks
        while i + 8 <= data.len() {
            let mut chunk = [0u8; 8];
            chunk.copy_from_slice(&data[i..i + 8]);
            let x = u64::from_le_bytes(chunk);

            // Construct big-endian u16 words from byte pairs.
            let lo = x & 0x00ff00ff00ff00ff;
            let hi = (x & 0xff00ff00ff00ff00) >> 8;
            let words = (lo << 8) + hi;

            sum += (words & 0xffff) as u64;
            sum += ((words >> 16) & 0xffff) as u64;
            sum += ((words >> 32) & 0xffff) as u64;
            sum += ((words >> 48) & 0xffff) as u64;
            i += 8;
        }

        // Remaining 16-bit words
        while i + 1 < data.len() {
            sum += u16::from_be_bytes([data[i], data[i + 1]]) as u64;
            i += 2;
        }
        // Odd trailing byte
        if i < data.len() {
            sum += u16::from_be_bytes([data[i], 0]) as u64;
        }
        sum
    }
    #[cfg(not(target_endian = "little"))]
    {
        let mut sum = 0u64;
        let mut i = 0usize;
        while i + 1 < data.len() {
            sum += u16::from_be_bytes([data[i], data[i + 1]]) as u64;
            i += 2;
        }
        if i < data.len() {
            sum += u16::from_be_bytes([data[i], 0]) as u64;
        }
        sum
    }
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
