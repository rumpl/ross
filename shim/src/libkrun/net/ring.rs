//! Simple bounded packet ring used by the multi-threaded network stack.
//!
//! The stack’s concurrency model is many-producer/consumer across threads, so we use
//! a mutex-protected bounded queue. This is intentionally simple and correct; if we
//! need more throughput later we can swap it for a lock-free structure.

use std::collections::VecDeque;
use std::sync::Mutex;

/// Bounded FIFO queue of variable-length packets.
pub struct PacketRing {
    inner: Mutex<VecDeque<Vec<u8>>>,
    cap_packets: usize,
}

impl PacketRing {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            // Avoid unbounded memory growth under backpressure.
            cap_packets: 2048,
        }
    }

    /// Push a packet into the ring.
    /// Returns `false` if the ring is full (caller may retry/backpressure).
    pub fn push(&self, pkt: &[u8]) -> bool {
        let mut q = self.inner.lock().expect("packet ring mutex poisoned");
        if q.len() >= self.cap_packets {
            return false;
        }
        q.push_back(pkt.to_vec());
        true
    }

    /// Pop one packet from the ring into `out`, returning the packet length.
    pub fn pop(&self, out: &mut [u8]) -> Option<usize> {
        let mut q = self.inner.lock().expect("packet ring mutex poisoned");
        let pkt = q.pop_front()?;
        let n = pkt.len().min(out.len());
        out[..n].copy_from_slice(&pkt[..n]);
        Some(n)
    }
}


