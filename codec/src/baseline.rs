//! Baseline history storage for snapshots.

use std::num::NonZeroUsize;

use crate::SnapshotTick;

/// Errors that can occur when inserting into the baseline store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaselineError {
    /// Ticks must be strictly increasing.
    OutOfOrder {
        last_tick: SnapshotTick,
        new_tick: SnapshotTick,
    },
}

/// A fixed-capacity ring buffer of baselines keyed by tick.
#[derive(Debug)]
pub struct BaselineStore<T> {
    entries: Vec<Option<Entry<T>>>,
    head: usize,
    len: usize,
    last_tick: Option<SnapshotTick>,
}

#[derive(Debug)]
struct Entry<T> {
    tick: SnapshotTick,
    value: T,
}

impl<T> BaselineStore<T> {
    /// Creates a new baseline store with the given capacity.
    #[must_use]
    pub fn new(capacity: NonZeroUsize) -> Self {
        let cap = capacity.get();
        let mut entries = Vec::with_capacity(cap);
        entries.resize_with(cap, || None);
        Self {
            entries,
            head: 0,
            len: 0,
            last_tick: None,
        }
    }

    /// Returns the capacity of the store.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// Returns the number of entries stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Inserts a new baseline at the given tick.
    ///
    /// Ticks must be strictly increasing.
    ///
    /// When the store is full, this overwrites the oldest entry and advances
    /// the head, making the inserted tick the newest entry immediately.
    pub fn insert(&mut self, tick: SnapshotTick, value: T) -> Result<(), BaselineError> {
        if let Some(last) = self.last_tick {
            if tick <= last {
                return Err(BaselineError::OutOfOrder {
                    last_tick: last,
                    new_tick: tick,
                });
            }
        }

        let cap = self.entries.len();
        if self.len < cap {
            let idx = (self.head + self.len) % cap;
            self.entries[idx] = Some(Entry { tick, value });
            self.len += 1;
        } else {
            self.entries[self.head] = Some(Entry { tick, value });
            self.head = (self.head + 1) % cap;
        }

        self.last_tick = Some(tick);
        Ok(())
    }

    /// Returns the baseline for an exact tick, if present.
    #[must_use]
    pub fn get(&self, tick: SnapshotTick) -> Option<&T> {
        self.iter().find(|(t, _)| *t == tick).map(|(_, v)| v)
    }

    /// Returns the latest baseline at or before the given tick.
    ///
    /// This performs an O(capacity) scan and is intended for small windows.
    #[must_use]
    pub fn latest_at_or_before(&self, tick: SnapshotTick) -> Option<(SnapshotTick, &T)> {
        for (t, v) in self.iter().rev() {
            if t <= tick {
                return Some((t, v));
            }
        }
        None
    }

    /// Returns an iterator from oldest to newest.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (SnapshotTick, &T)> {
        let cap = self.entries.len();
        (0..self.len).filter_map(move |i| {
            let idx = (self.head + i) % cap;
            self.entries[idx]
                .as_ref()
                .map(|entry| (entry.tick, &entry.value))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_get() {
        let mut store = BaselineStore::new(NonZeroUsize::new(3).unwrap());
        store.insert(SnapshotTick::new(1), 10).unwrap();
        store.insert(SnapshotTick::new(2), 20).unwrap();

        assert_eq!(store.get(SnapshotTick::new(1)), Some(&10));
        assert_eq!(store.get(SnapshotTick::new(2)), Some(&20));
        assert_eq!(store.get(SnapshotTick::new(3)), None);
    }

    #[test]
    fn latest_at_or_before() {
        let mut store = BaselineStore::new(NonZeroUsize::new(3).unwrap());
        store.insert(SnapshotTick::new(10), 1).unwrap();
        store.insert(SnapshotTick::new(20), 2).unwrap();
        store.insert(SnapshotTick::new(30), 3).unwrap();

        assert_eq!(
            store
                .latest_at_or_before(SnapshotTick::new(25))
                .map(|(t, _)| t),
            Some(SnapshotTick::new(20))
        );
        assert_eq!(
            store
                .latest_at_or_before(SnapshotTick::new(30))
                .map(|(t, _)| t),
            Some(SnapshotTick::new(30))
        );
        assert_eq!(store.latest_at_or_before(SnapshotTick::new(5)), None);
    }

    #[test]
    fn evicts_oldest_when_full() {
        let mut store = BaselineStore::new(NonZeroUsize::new(2).unwrap());
        store.insert(SnapshotTick::new(1), 1).unwrap();
        store.insert(SnapshotTick::new(2), 2).unwrap();
        store.insert(SnapshotTick::new(3), 3).unwrap();

        assert_eq!(store.get(SnapshotTick::new(1)), None);
        assert_eq!(store.get(SnapshotTick::new(2)), Some(&2));
        assert_eq!(store.get(SnapshotTick::new(3)), Some(&3));
    }

    #[test]
    fn rejects_out_of_order_ticks() {
        let mut store = BaselineStore::new(NonZeroUsize::new(2).unwrap());
        store.insert(SnapshotTick::new(10), 1).unwrap();
        let err = store.insert(SnapshotTick::new(9), 2).unwrap_err();
        assert!(matches!(err, BaselineError::OutOfOrder { .. }));
    }

    #[test]
    fn lookup_after_wraparound() {
        let mut store = BaselineStore::new(NonZeroUsize::new(3).unwrap());
        store.insert(SnapshotTick::new(1), 1).unwrap();
        store.insert(SnapshotTick::new(2), 2).unwrap();
        store.insert(SnapshotTick::new(3), 3).unwrap();
        store.insert(SnapshotTick::new(4), 4).unwrap();
        store.insert(SnapshotTick::new(5), 5).unwrap();

        assert_eq!(store.get(SnapshotTick::new(1)), None);
        assert_eq!(store.get(SnapshotTick::new(2)), None);
        assert_eq!(store.get(SnapshotTick::new(3)), Some(&3));
        assert_eq!(store.get(SnapshotTick::new(4)), Some(&4));
        assert_eq!(store.get(SnapshotTick::new(5)), Some(&5));
    }

    #[test]
    fn latest_at_or_before_across_eviction() {
        let mut store = BaselineStore::new(NonZeroUsize::new(3).unwrap());
        store.insert(SnapshotTick::new(10), 1).unwrap();
        store.insert(SnapshotTick::new(11), 2).unwrap();
        store.insert(SnapshotTick::new(12), 3).unwrap();
        store.insert(SnapshotTick::new(13), 4).unwrap();

        assert_eq!(store.latest_at_or_before(SnapshotTick::new(10)), None);
        assert_eq!(
            store
                .latest_at_or_before(SnapshotTick::new(11))
                .map(|(t, _)| t),
            Some(SnapshotTick::new(11))
        );
        assert_eq!(
            store
                .latest_at_or_before(SnapshotTick::new(100))
                .map(|(t, _)| t),
            Some(SnapshotTick::new(13))
        );
    }

    #[test]
    fn stress_insert_wraparound() {
        let mut store = BaselineStore::new(NonZeroUsize::new(3).unwrap());
        for tick in 1..=50 {
            store.insert(SnapshotTick::new(tick), tick).unwrap();
        }

        assert_eq!(store.len(), 3);
        assert_eq!(store.latest_at_or_before(SnapshotTick::new(1)), None);
        assert_eq!(
            store
                .latest_at_or_before(SnapshotTick::new(48))
                .map(|(t, _)| t),
            Some(SnapshotTick::new(48))
        );
        assert_eq!(
            store
                .latest_at_or_before(SnapshotTick::new(49))
                .map(|(t, _)| t),
            Some(SnapshotTick::new(49))
        );
        assert_eq!(
            store
                .latest_at_or_before(SnapshotTick::new(50))
                .map(|(t, _)| t),
            Some(SnapshotTick::new(50))
        );
        assert_eq!(store.get(SnapshotTick::new(47)), None);
    }
}
