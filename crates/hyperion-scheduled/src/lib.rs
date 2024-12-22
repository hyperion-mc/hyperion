//! A priority queue that allows popping items until a given key.

use std::{
    borrow::Borrow,
    cmp::{Ordering, Reverse},
    collections::{BinaryHeap, binary_heap::PeekMut},
};

/// Internal type for storing key-value pairs in the priority queue
struct KeyValue<K, V>(K, V);

impl<K: Ord, V> Eq for KeyValue<K, V> {}

impl<K: Ord, V> PartialEq for KeyValue<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<K: Ord, V> Ord for KeyValue<K, V> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl<K: Ord, V> PartialOrd for KeyValue<K, V> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A priority queue that stores key-value pairs and allows popping items until a given key.
/// The queue is ordered by the keys in ascending order.
pub struct Scheduled<K, V> {
    // min-heap
    queue: BinaryHeap<Reverse<KeyValue<K, V>>>,
}

impl<K: Ord, V> Default for Scheduled<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Ord, V> Scheduled<K, V> {
    /// Creates a new empty [`Scheduled`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            queue: BinaryHeap::new(),
        }
    }

    /// Adds a key-value pair to the queue.
    pub fn schedule(&mut self, key: K, value: V) {
        self.queue.push(Reverse(KeyValue(key, value)));
    }

    /// Returns an iterator that yields values with keys less than or equal to the given limit.
    /// The values are yielded in ascending order of their keys.
    pub fn pop_until<'a, Q>(&'a mut self, limit: &'a Q) -> impl Iterator<Item = V> + 'a
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        std::iter::from_fn(move || {
            let peek = self.queue.peek_mut()?;

            let Reverse(KeyValue(key, _)) = &*peek;
            (key.borrow() <= limit).then(|| {
                let Reverse(KeyValue(_, value)) = PeekMut::pop(peek);
                value
            })
        })
    }

    /// Removes all items from the queue.
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// Returns a reference to the key-value pair with the smallest key, if any.
    #[must_use]
    pub fn peek(&self) -> Option<(&K, &V)> {
        self.queue
            .peek()
            .map(|Reverse(KeyValue(key, value))| (key, value))
    }

    /// Returns `true` if the queue contains no items.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Returns the number of items in the queue.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
#[allow(clippy::needless_collect)]
mod tests {
    use super::*;

    #[test]
    fn test_new_and_is_empty() {
        let data: Scheduled<i32, &str> = Scheduled::new();
        assert!(data.is_empty());
        assert_eq!(data.len(), 0);
    }

    #[test]
    fn test_schedule_and_peek() {
        let mut data = Scheduled::new();
        data.schedule(3, "three");
        data.schedule(1, "one");
        data.schedule(2, "two");

        assert_eq!(data.peek(), Some((&1, &"one")));
        assert_eq!(data.len(), 3);
    }

    #[test]
    fn test_pop_all_before() {
        let mut data = Scheduled::new();
        data.schedule(3, "three");
        data.schedule(1, "one");
        data.schedule(2, "two");
        data.schedule(4, "four");

        let result: Vec<_> = data.pop_until(&2).collect();
        assert_eq!(result, vec!["one", "two"]);
        assert_eq!(data.len(), 2);
        assert_eq!(data.peek(), Some((&3, &"three")));
    }

    #[test]
    fn test_clear() {
        let mut data = Scheduled::new();
        data.schedule(1, "one");
        data.schedule(2, "two");
        assert_eq!(data.len(), 2);

        data.clear();
        assert!(data.is_empty());
        assert_eq!(data.len(), 0);
    }

    #[test]
    fn test_pop_all_before_empty() {
        let mut data: Scheduled<i32, &str> = Scheduled::new();
        let result: Vec<_> = data.pop_until(&5).collect();
        assert!(result.is_empty());
    }

    #[test]
    fn test_pop_all_before_no_items() {
        let mut data = Scheduled::new();
        data.schedule(10, "ten");
        data.schedule(20, "twenty");

        let result: Vec<_> = data.pop_until(&5).collect();
        assert!(result.is_empty());
        assert_eq!(data.len(), 2);
    }

    #[test]
    fn test_ordering() {
        let mut data = Scheduled::new();
        data.schedule(3, "three");
        data.schedule(1, "one");
        data.schedule(2, "two");

        assert_eq!(data.pop_until(&4).collect::<Vec<_>>(), vec![
            "one", "two", "three"
        ]);

        let mut data = Scheduled::new();
        data.schedule(3, "three");
        data.schedule(1, "one");
        data.schedule(2, "two");

        assert_eq!(data.pop_until(&1).collect::<Vec<_>>(), vec!["one"]);
    }
}
