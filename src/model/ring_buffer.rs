use std::collections::VecDeque;

/// Default capacity: 600 samples (10 minutes at 1 Hz).
///
/// This is a provisional default; see open question Q5 in
/// `docs/engineering-contract.md`.
pub const CAPACITY: usize = 600;

/// Fixed-capacity ring buffer.
///
/// When `push` is called on a full buffer the oldest element is evicted to make
/// room for the new one.  The ordering of the iterator is oldest-first
/// (insertion order).
#[derive(Debug, Clone)]
pub struct RingBuffer<T> {
    inner: VecDeque<T>,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    /// Create a new ring buffer with the default capacity (`CAPACITY`).
    pub fn new() -> Self {
        Self::with_capacity(CAPACITY)
    }

    /// Create a new ring buffer with the given capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity == 0`.
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity > 0, "RingBuffer capacity must be > 0");
        Self {
            inner: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push a value into the buffer, evicting the oldest entry if the buffer
    /// is already at capacity.
    pub fn push(&mut self, value: T) {
        if self.inner.len() == self.capacity {
            self.inner.pop_front();
        }
        self.inner.push_back(value);
    }

    /// Returns the number of elements currently stored.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` when no elements are stored.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns `true` when the buffer holds exactly `capacity` elements.
    #[allow(dead_code)]
    pub fn is_full(&self) -> bool {
        self.inner.len() == self.capacity
    }

    /// The fixed capacity of the buffer.
    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Iterate from oldest to newest.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.inner.iter()
    }

    /// Return a reference to the most recently pushed element, or `None` if
    /// empty.
    #[allow(dead_code)]
    pub fn latest(&self) -> Option<&T> {
        self.inner.back()
    }
}

impl<T> Default for RingBuffer<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_len() {
        let mut rb: RingBuffer<u32> = RingBuffer::with_capacity(4);
        assert_eq!(rb.len(), 0);
        assert!(rb.is_empty());

        rb.push(1);
        rb.push(2);
        assert_eq!(rb.len(), 2);
        assert!(!rb.is_empty());
        assert!(!rb.is_full());
    }

    #[test]
    fn fills_to_capacity() {
        let mut rb: RingBuffer<u32> = RingBuffer::with_capacity(3);
        rb.push(10);
        rb.push(20);
        rb.push(30);
        assert!(rb.is_full());
        assert_eq!(rb.len(), 3);
    }

    #[test]
    fn overflow_evicts_oldest() {
        let mut rb: RingBuffer<u32> = RingBuffer::with_capacity(3);
        rb.push(1);
        rb.push(2);
        rb.push(3);
        // Buffer is full; push a fourth element — oldest (1) should be evicted.
        rb.push(4);
        assert_eq!(rb.len(), 3);
        let items: Vec<u32> = rb.iter().copied().collect();
        assert_eq!(items, vec![2, 3, 4]);
    }

    #[test]
    fn overflow_multiple_evictions() {
        let mut rb: RingBuffer<u32> = RingBuffer::with_capacity(3);
        for i in 1..=6 {
            rb.push(i);
        }
        // Only the last 3 should remain.
        let items: Vec<u32> = rb.iter().copied().collect();
        assert_eq!(items, vec![4, 5, 6]);
    }

    #[test]
    fn iteration_order_is_oldest_first() {
        let mut rb: RingBuffer<u32> = RingBuffer::with_capacity(5);
        for i in 0..5 {
            rb.push(i);
        }
        let items: Vec<u32> = rb.iter().copied().collect();
        assert_eq!(items, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn latest_returns_most_recent() {
        let mut rb: RingBuffer<u32> = RingBuffer::with_capacity(4);
        assert_eq!(rb.latest(), None);
        rb.push(7);
        rb.push(8);
        assert_eq!(rb.latest(), Some(&8));
    }

    #[test]
    #[should_panic(expected = "capacity must be > 0")]
    fn zero_capacity_panics() {
        let _: RingBuffer<u32> = RingBuffer::with_capacity(0);
    }
}
