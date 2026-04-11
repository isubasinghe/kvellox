use std::borrow::Borrow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;

/// S3-FIFO Cache implementation.
/// Based on "S3-FIFO: Simple, Scalable, and Static FIFO Queues for Web Cache Workloads"
/// (https://dl.acm.org/doi/10.1145/3600006.3613147)
pub struct S3Fifo<K, V> {
    capacity: usize,
    items: HashMap<K, Entry<V>>,
    small: VecDeque<K>,
    main: VecDeque<K>,
    /// Ghost contains keys evicted from Small that were "one-hit wonders" (never accessed after insert)
    /// but need to be tracked for "early exit" detection.
    ghost_set: HashSet<K>,
    ghost_queue: VecDeque<K>,
}

struct Entry<V> {
    value: V,
    /// Frequency counter (0-3)
    freq: u8,
}

impl<K, V> S3Fifo<K, V>
where
    K: Clone + Hash + Eq,
{
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "Capacity must be greater than 0");
        Self {
            capacity,
            items: HashMap::with_capacity(capacity),
            small: VecDeque::new(),
            main: VecDeque::new(),
            ghost_set: HashSet::new(),
            ghost_queue: VecDeque::new(),
        }
    }

    /// Get a reference to the value associated with the key.
    /// Updates frequency on hit.
    pub fn get<Q>(&mut self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if let Some(entry) = self.items.get_mut(key) {
            entry.freq = (entry.freq + 1).min(3);
            return Some(&entry.value);
        }
        None
    }

    /// Get a mutable reference to the value associated with the key.
    /// Updates frequency on hit.
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if let Some(entry) = self.items.get_mut(key) {
            entry.freq = (entry.freq + 1).min(3);
            return Some(&mut entry.value);
        }
        None
    }

    /// Insert a key-value pair into the cache.
    pub fn put(&mut self, key: K, value: V) {
        if let Some(entry) = self.items.get_mut(&key) {
            entry.value = value;
            entry.freq = (entry.freq + 1).min(3);
            return;
        }

        while self.items.len() >= self.capacity {
            self.evict();
        }

        // Insert new item
        // Check Ghost
        if self.ghost_set.contains(&key) {
            // Hit in Ghost: Promote to Main
            self.ghost_set.remove(&key);
            // We leave the key in ghost_queue, it will be cleaned up lazily
            
            self.main.push_back(key.clone());
            self.items.insert(key, Entry { value, freq: 0 });
        } else {
            // Insert to Small
            self.small.push_back(key.clone());
            self.items.insert(key, Entry { value, freq: 0 });
        }
    }

    /// Check if the cache contains a key without updating frequency.
    pub fn contains<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.items.contains_key(key)
    }

    /// Return the number of items in the cache.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Return true if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn evict(&mut self) {
        let small_threshold = self.capacity / 10;

        loop {
            if self.small.len() > small_threshold || (self.main.is_empty() && !self.small.is_empty()) {
                if self.small.is_empty() {
                    return;
                }

                let key = self.small.pop_front().unwrap();
                let freq = self.items.get(&key).unwrap().freq;

                if freq > 0 {
                    // Promote to Main
                    self.items.get_mut(&key).unwrap().freq = 0;
                    self.main.push_back(key);
                } else {
                    // Evict from Small → Ghost
                    self.items.remove(&key);
                    self.insert_ghost(key);
                    return;
                }
            } else {
                if self.main.is_empty() {
                    return;
                }

                let key = self.main.pop_front().unwrap();
                let freq = self.items.get(&key).unwrap().freq;

                if freq > 0 {
                    self.items.get_mut(&key).unwrap().freq -= 1;
                    self.main.push_back(key);
                } else {
                    // Evict from Main (no Ghost)
                    self.items.remove(&key);
                    return;
                }
            }
        }
    }

    fn insert_ghost(&mut self, key: K) {
        if self.capacity == 0 { return; }
        
        self.ghost_set.insert(key.clone());
        self.ghost_queue.push_back(key);

        // Maintain Ghost size <= Capacity
        while self.ghost_set.len() > self.capacity {
             if let Some(k) = self.ghost_queue.pop_front() {
                 if self.ghost_set.contains(&k) {
                     self.ghost_set.remove(&k);
                 }
                 // If not in set, it was already promoted/removed, ignore.
             } else {
                 break;
             }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_put_get() {
        let mut cache = S3Fifo::new(2);
        cache.put("a", 1);
        assert_eq!(cache.get("a"), Some(&1));
        cache.put("b", 2);
        assert_eq!(cache.get("b"), Some(&2));
    }

    #[test]
    fn test_eviction_fifo_behavior_on_small() {
        // Capacity 2. Threshold = 0.
        // Insert A, B. S=[A, B].
        // Insert C. Evict?
        // S > 0. Pop A. Freq 0 (no access). Evict A. Ghost={A}. S=[B], M=[]. Items={B, C}.
        let mut cache = S3Fifo::new(2);
        cache.put("a", 1);
        cache.put("b", 2);
        cache.put("c", 3);

        assert_eq!(cache.get("a"), None); // A evicted
        assert_eq!(cache.get("b"), Some(&2));
        assert_eq!(cache.get("c"), Some(&3));
    }

    #[test]
    fn test_promotion_to_main() {
        // Cap 2.
        // Insert A. S=[A].
        // Get A. A.freq=1.
        // Insert B. S=[A, B].
        // Insert C. Evict.
        // S check: Pop A. Freq=1. Move A to M. A.freq=0. M=[A]. S=[B].
        // Loop. S check. Pop B. Freq=0. Evict B. Ghost={B}.
        // Result: A kept (in M), B evicted, C inserted.
        let mut cache = S3Fifo::new(2);
        cache.put("a", 1);
        cache.get("a"); // A promoted logic trigger upon eviction time
        cache.put("b", 2);
        cache.put("c", 3);

        assert_eq!(cache.get("a"), Some(&1)); // A should be there
        assert_eq!(cache.get("b"), None); // B evicted
        assert_eq!(cache.get("c"), Some(&3));
    }

    #[test]
    fn test_ghost_hit_promotion() {
        // Cap 2.
        // Insert A, B. S=[A, B].
        // Insert C. A evicted (freq 0). Ghost={A}.
        // Put A again. Ghost hit!
        // A inserted to M. freq=0.
        // Insert D. Evict.
        // S=[B, C]. M=[A].
        // Evict S: Pop B (freq 0). Evict B.
        let mut cache = S3Fifo::new(2);
        cache.put("a", 1);
        cache.put("b", 2);
        cache.put("c", 3);
        
        assert_eq!(cache.get("a"), None); // A evicted
        
        // Reinsert A (Ghost hit)
        cache.put("a", 10);
        // A should be in Main now.

        // Insert D. Should evict B (from Small) before A (from Main).
        cache.put("d", 4);

        assert_eq!(cache.get("b"), None);
        assert_eq!(cache.get("a"), Some(&10));
        assert_eq!(cache.get("d"), Some(&4));
    }
}
