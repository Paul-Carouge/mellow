//! A least-recently-used map bounded by the bytes its values hold, shared by the three picture
//! caches (decoded pictures, attachment files in memory, the disk cache's index).

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

#[derive(Debug)]
struct Entry<V> {
    value: V,
    byte_count: usize,
    last_use: u64,
}

#[derive(Debug)]
pub(crate) struct ByteBoundedLru<K, V> {
    capacity_bytes: usize,
    used_bytes: usize,
    next_use: u64,
    entries: HashMap<K, Entry<V>>,
    /// Keys by their last use, oldest first: eviction takes the first.
    keys_by_last_use: BTreeMap<u64, K>,
}

impl<K: Eq + Hash + Clone, V> ByteBoundedLru<K, V> {
    pub(crate) fn new(capacity_bytes: usize) -> Self {
        Self {
            capacity_bytes,
            used_bytes: 0,
            next_use: 0,
            entries: HashMap::new(),
            keys_by_last_use: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    fn take_next_use(&mut self) -> u64 {
        let use_number = self.next_use;
        self.next_use += 1;
        use_number
    }

    /// The value, marked as the most recently used.
    pub(crate) fn get(&mut self, key: &K) -> Option<&V> {
        let use_number = self.take_next_use();
        let entry = self.entries.get_mut(key)?;
        self.keys_by_last_use.remove(&entry.last_use);
        self.keys_by_last_use.insert(use_number, key.clone());
        entry.last_use = use_number;
        Some(&entry.value)
    }

    /// Stores `value` and returns what had to leave to make room: older entries, the value this
    /// key held before, or `value` itself when it alone is larger than the whole capacity.
    pub(crate) fn insert(&mut self, key: K, value: V, byte_count: usize) -> Vec<(K, V)> {
        let mut evicted: Vec<(K, V)> = self
            .remove(&key)
            .map(|old| (key.clone(), old))
            .into_iter()
            .collect();
        if byte_count > self.capacity_bytes {
            evicted.push((key, value));
            return evicted;
        }
        while self.used_bytes + byte_count > self.capacity_bytes {
            let Some((_, oldest_key)) = self.keys_by_last_use.pop_first() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest_key) {
                self.used_bytes -= entry.byte_count;
                evicted.push((oldest_key, entry.value));
            }
        }
        let use_number = self.take_next_use();
        self.keys_by_last_use.insert(use_number, key.clone());
        self.used_bytes += byte_count;
        self.entries.insert(
            key,
            Entry {
                value,
                byte_count,
                last_use: use_number,
            },
        );
        evicted
    }

    pub(crate) fn remove(&mut self, key: &K) -> Option<V> {
        let entry = self.entries.remove(key)?;
        self.keys_by_last_use.remove(&entry.last_use);
        self.used_bytes -= entry.byte_count;
        Some(entry.value)
    }

    /// Empties the map and hands every value back, for a caller that must release them.
    pub(crate) fn take_all(&mut self) -> Vec<(K, V)> {
        self.keys_by_last_use.clear();
        self.used_bytes = 0;
        self.entries
            .drain()
            .map(|(key, entry)| (key, entry.value))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(evicted: Vec<(&'static str, u8)>) -> Vec<&'static str> {
        evicted.into_iter().map(|(key, _)| key).collect()
    }

    #[test]
    fn the_least_recently_used_entries_leave_first_once_the_bytes_are_spent() {
        let mut cache = ByteBoundedLru::new(10);
        assert_eq!(cache.insert("a", 1, 4), Vec::new());
        assert_eq!(cache.insert("b", 2, 4), Vec::new());
        // Reading `a` makes `b` the oldest.
        assert_eq!(cache.get(&"a"), Some(&1));
        assert_eq!(keys(cache.insert("c", 3, 4)), ["b"]);
        assert_eq!(cache.used_bytes(), 8);
        assert_eq!(cache.get(&"b"), None);
        assert_eq!(keys(cache.insert("d", 4, 10)), ["a", "c"]);
        assert_eq!(cache.used_bytes(), 10);
    }

    #[test]
    fn a_value_larger_than_the_capacity_is_handed_back_and_evicts_nothing() {
        let mut cache = ByteBoundedLru::new(10);
        cache.insert("a", 1, 6);
        assert_eq!(cache.insert("huge", 2, 11), [("huge", 2)]);
        assert_eq!(cache.get(&"a"), Some(&1));
    }

    #[test]
    fn replacing_a_key_returns_its_old_value_and_counts_bytes_once() {
        let mut cache = ByteBoundedLru::new(10);
        cache.insert("a", 1, 6);
        assert_eq!(cache.insert("a", 2, 7), [("a", 1)]);
        assert_eq!(cache.used_bytes(), 7);
        assert_eq!(cache.remove(&"a"), Some(2));
        assert_eq!(cache.used_bytes(), 0);
    }

    #[test]
    fn taking_everything_empties_the_map() {
        let mut cache = ByteBoundedLru::new(10);
        cache.insert("a", 1, 3);
        cache.insert("b", 2, 3);
        let mut taken = keys(cache.take_all());
        taken.sort_unstable();
        assert_eq!(taken, ["a", "b"]);
        assert_eq!(cache.used_bytes(), 0);
        assert_eq!(cache.insert("c", 3, 10), Vec::new());
    }
}
