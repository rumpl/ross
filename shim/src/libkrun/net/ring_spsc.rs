//! Lock-free single-producer / single-consumer packet ring.
//!
//! This is intentionally SPSC: in `stack.rs` we allocate one ring per worker for RX and TX:
//! - RX ring: producer = main thread, consumer = worker thread
//! - TX ring: producer = worker thread, consumer = main thread
//!
//! That makes SPSC safe and very fast (no mutex, no per-packet heap allocation).

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Max packet size supported by the ring.
const MAX_PACKET: usize = 65535;

/// Number of slots (power of two).
///
/// Note: memory usage is roughly `RING_SIZE * MAX_PACKET` bytes per ring. Since we
/// allocate 2 rings per worker (RX + TX), keep this reasonably small.
const RING_SIZE: usize = 256;

#[repr(C, align(64))]
struct CacheAligned<T>(T);

/// A lock-free SPSC ring of packets.
#[repr(C)]
pub struct SpscPacketRing {
    head: CacheAligned<AtomicU64>,
    tail: CacheAligned<AtomicU64>,
    lens: Box<[AtomicU32]>,
    data: Box<[u8]>,
}

impl SpscPacketRing {
    pub fn new() -> Self {
        // IMPORTANT: avoid constructing a gigantic `[Slot; RING_SIZE]` on the stack.
        // Allocate backing storage directly on the heap.
        let lens: Vec<AtomicU32> = (0..RING_SIZE).map(|_| AtomicU32::new(0)).collect();
        let data = vec![0u8; RING_SIZE * MAX_PACKET];
        Self {
            head: CacheAligned(AtomicU64::new(0)),
            tail: CacheAligned(AtomicU64::new(0)),
            lens: lens.into_boxed_slice(),
            data: data.into_boxed_slice(),
        }
    }

    /// Try to push a packet; returns false if full or too large.
    #[inline]
    pub fn push(&self, pkt: &[u8]) -> bool {
        if pkt.len() > MAX_PACKET {
            return false;
        }

        let head = self.head.0.load(Ordering::Relaxed);
        let tail = self.tail.0.load(Ordering::Acquire);

        if head.wrapping_sub(tail) >= RING_SIZE as u64 {
            return false;
        }

        let idx = (head as usize) & (RING_SIZE - 1);
        let off = idx * MAX_PACKET;
        // Safety: SPSC ring - only the producer thread writes to slot `idx` before publishing head.
        unsafe {
            let dst = self.data.as_ptr().add(off) as *mut u8;
            std::ptr::copy_nonoverlapping(pkt.as_ptr(), dst, pkt.len());
        }
        // Publish len before releasing head.
        self.lens[idx].store(pkt.len() as u32, Ordering::Relaxed);

        self.head.0.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// Try to pop a packet into `out`; returns the packet length.
    #[inline]
    pub fn pop(&self, out: &mut [u8]) -> Option<usize> {
        let tail = self.tail.0.load(Ordering::Relaxed);
        let head = self.head.0.load(Ordering::Acquire);
        if tail == head {
            return None;
        }

        let idx = (tail as usize) & (RING_SIZE - 1);
        let len = self.lens[idx].load(Ordering::Relaxed) as usize;
        if len > out.len() {
            self.tail.0.store(tail.wrapping_add(1), Ordering::Release);
            return None;
        }

        let off = idx * MAX_PACKET;
        // Safety: SPSC ring - only the consumer reads from slot `idx` after observing head via Acquire.
        unsafe {
            let src = self.data.as_ptr().add(off);
            std::ptr::copy_nonoverlapping(src, out.as_mut_ptr(), len);
        }
        self.tail.0.store(tail.wrapping_add(1), Ordering::Release);
        Some(len)
    }
}

unsafe impl Send for SpscPacketRing {}
unsafe impl Sync for SpscPacketRing {}


