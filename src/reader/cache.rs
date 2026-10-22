//! Begrensde LRU-cache van recent gelezen recordbytes (task.md T2, blueprint.md
//! `reader::cache`). De offset-index zelf bevat nooit inhoud ("Blijft" in T2); deze
//! cache is de enige plek waar recordbytes tijdelijk in het geheugen staan, en dat
//! begrensd op een vast aantal records.

use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

/// Standaardcapaciteit uit task.md T2: "een kleine LRU-cache (standaard 4096
/// records)".
pub const DEFAULT_CAPACITY: usize = 4096;

/// Sleutel = startoffset van het record (blueprint.md `reader::cache`).
pub struct RecordCache {
    inner: Mutex<LruCache<u64, Arc<[u8]>>>,
}

impl RecordCache {
    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::MIN);
        Self {
            inner: Mutex::new(LruCache::new(capacity)),
        }
    }

    pub fn get(&self, offset: u64) -> Option<Arc<[u8]>> {
        self.inner
            .lock()
            .expect("cache-lock vergiftigd")
            .get(&offset)
            .cloned()
    }

    pub fn insert(&self, offset: u64, bytes: Arc<[u8]>) {
        self.inner
            .lock()
            .expect("cache-lock vergiftigd")
            .put(offset, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_then_get_returns_same_bytes() {
        let cache = RecordCache::with_capacity(2);
        let bytes: Arc<[u8]> = Arc::from(b"hallo".as_slice());
        cache.insert(0, Arc::clone(&bytes));
        assert_eq!(cache.get(0), Some(bytes));
        assert_eq!(cache.get(42), None);
    }

    #[test]
    fn capacity_is_bounded() {
        let cache = RecordCache::with_capacity(1);
        cache.insert(0, Arc::from(b"a".as_slice()));
        cache.insert(1, Arc::from(b"b".as_slice()));
        // Capaciteit 1: de oudste sleutel (0) is verdrongen.
        assert_eq!(cache.get(0), None);
        assert!(cache.get(1).is_some());
    }
}
