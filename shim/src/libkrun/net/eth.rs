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
/// Heavily optimized for high-throughput networking:
/// - Processes 32 bytes at a time on the main loop
/// - Uses SIMD-friendly operations that the compiler can auto-vectorize
#[inline]
fn sum_be_words(data: &[u8]) -> u64 {
    let mut sum = 0u64;
    let mut i = 0usize;
    let len = data.len();
    
    // Process 32 bytes at a time for better cache utilization and potential vectorization
    while i + 32 <= len {
        // Load 4 u64s at once - this pattern is friendly to auto-vectorization
        let a = u64::from_ne_bytes([
            data[i], data[i+1], data[i+2], data[i+3],
            data[i+4], data[i+5], data[i+6], data[i+7]
        ]);
        let b = u64::from_ne_bytes([
            data[i+8], data[i+9], data[i+10], data[i+11],
            data[i+12], data[i+13], data[i+14], data[i+15]
        ]);
        let c = u64::from_ne_bytes([
            data[i+16], data[i+17], data[i+18], data[i+19],
            data[i+20], data[i+21], data[i+22], data[i+23]
        ]);
        let d = u64::from_ne_bytes([
            data[i+24], data[i+25], data[i+26], data[i+27],
            data[i+28], data[i+29], data[i+30], data[i+31]
        ]);
        
        // Sum all u16 words - use horizontal add pattern
        sum += sum_u64_be_words(a);
        sum += sum_u64_be_words(b);
        sum += sum_u64_be_words(c);
        sum += sum_u64_be_words(d);
        i += 32;
    }
    
    // Process remaining 8-byte chunks
    while i + 8 <= len {
        let x = u64::from_ne_bytes([
            data[i], data[i+1], data[i+2], data[i+3],
            data[i+4], data[i+5], data[i+6], data[i+7]
        ]);
        sum += sum_u64_be_words(x);
        i += 8;
    }

    // Remaining 16-bit words
    while i + 1 < len {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u64;
        i += 2;
    }
    // Odd trailing byte
    if i < len {
        sum += u16::from_be_bytes([data[i], 0]) as u64;
    }
    sum
}

/// Sum four big-endian u16 words packed into a u64.
#[inline(always)]
fn sum_u64_be_words(x: u64) -> u64 {
    // For big-endian u16 words, we need to byte-swap pairs
    #[cfg(target_endian = "little")]
    {
        let lo = x & 0x00ff00ff00ff00ff;
        let hi = (x >> 8) & 0x00ff00ff00ff00ff;
        let swapped = (lo << 8) | hi;
        // Horizontal sum of four u16 words
        let w0 = swapped & 0xffff;
        let w1 = (swapped >> 16) & 0xffff;
        let w2 = (swapped >> 32) & 0xffff;
        let w3 = (swapped >> 48) & 0xffff;
        w0 + w1 + w2 + w3
    }
    #[cfg(target_endian = "big")]
    {
        let w0 = x & 0xffff;
        let w1 = (x >> 16) & 0xffff;
        let w2 = (x >> 32) & 0xffff;
        let w3 = (x >> 48) & 0xffff;
        w0 + w1 + w2 + w3
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
