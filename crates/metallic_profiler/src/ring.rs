//! A fixed-capacity ring of the most recent items.

/// Keeps the last `capacity` items pushed; a push into a full ring replaces the oldest.
///
/// The storage is reserved once by [`RingBuffer::new`]; pushing never allocates.
#[derive(Debug, Clone)]
pub struct RingBuffer<T> {
    items: Vec<T>,
    /// Where the next push goes once the ring is full (the oldest item).
    next: usize,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    /// An empty ring that keeps up to `capacity` items (at least 1).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        RingBuffer {
            items: Vec::with_capacity(capacity),
            next: 0,
            capacity,
        }
    }

    /// Adds `item` as the newest, dropping the oldest when the ring is full.
    pub fn push(&mut self, item: T) {
        if self.items.len() < self.capacity {
            self.items.push(item);
        } else {
            self.items[self.next] = item;
        }
        self.next = (self.next + 1) % self.capacity;
    }

    /// How many items it holds.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when it holds nothing.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The most items it keeps.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The items from the oldest to the newest.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        let split = if self.items.len() < self.capacity {
            0
        } else {
            self.next
        };
        let (newer, older) = self.items.split_at(split);
        older.iter().chain(newer.iter()).with_len(self.items.len())
    }

    /// The `i`-th item counting from the oldest (0).
    pub fn get(&self, i: usize) -> Option<&T> {
        if i >= self.items.len() {
            return None;
        }
        let start = if self.items.len() < self.capacity {
            0
        } else {
            self.next
        };
        self.items.get((start + i) % self.capacity)
    }

    /// The newest item.
    pub fn latest(&self) -> Option<&T> {
        self.len().checked_sub(1).and_then(|i| self.get(i))
    }

    /// The items from the newest back, to change them in place.
    pub(crate) fn newest_first_mut(&mut self) -> impl Iterator<Item = &mut T> {
        let split = if self.items.len() < self.capacity {
            self.items.len()
        } else {
            self.next
        };
        let (newer, older) = self.items.split_at_mut(split);
        newer.iter_mut().rev().chain(older.iter_mut().rev())
    }

    /// Forgets every item and keeps the storage.
    pub fn clear(&mut self) {
        self.items.clear();
        self.next = 0;
    }
}

/// An iterator that knows its exact length (a chain of two slices).
struct Exact<I> {
    inner: I,
    left: usize,
}

impl<I: Iterator> Iterator for Exact<I> {
    type Item = I::Item;
    fn next(&mut self) -> Option<I::Item> {
        let item = self.inner.next()?;
        self.left -= 1;
        Some(item)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl<I: DoubleEndedIterator> DoubleEndedIterator for Exact<I> {
    fn next_back(&mut self) -> Option<I::Item> {
        let item = self.inner.next_back()?;
        self.left -= 1;
        Some(item)
    }
}

impl<I: Iterator> ExactSizeIterator for Exact<I> {}

trait WithLen: Sized {
    fn with_len(self, len: usize) -> Exact<Self>;
}

impl<I: Iterator> WithLen for I {
    fn with_len(self, len: usize) -> Exact<Self> {
        Exact {
            inner: self,
            left: len,
        }
    }
}
