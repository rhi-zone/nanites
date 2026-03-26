//! Pluggable result cache for task outputs.
//!
//! The cache is keyed on a triple of (task type name, params snapshot, input
//! hash). Only tasks that implement [`crate::registry::SerializableTask`] and
//! whose inputs are [`serde::Serialize`] can produce a stable key; all other
//! tasks bypass the cache transparently.
//!
//! # Built-in implementations
//!
//! - [`NoCache`] — always misses. This is the default; zero overhead.
//! - [`MemoryCache`] — `Arc<Mutex<HashMap<…>>>`, no eviction.
//!
//! # Cloneability requirement
//!
//! [`AnyOutput`] is `Box<dyn Any + Send>`, which is not `Clone`. The cache
//! stores a [`CachedOutput`] wrapper that pairs the value with a clone
//! function. The runtime only stores outputs of tasks whose concrete output
//! type is registered as cloneable via [`cache_output`]. For any task whose
//! output type is not cloneable, caching is skipped.
//!
//! In practice, tasks opt in to caching by using [`cache_output`] on their
//! output before returning, which is called automatically by the runtime for
//! tasks implementing [`crate::registry::SerializableTask`].
//!
//! # Usage
//!
//! ```no_run
//! use nanites_core::{Runtime, cache::MemoryCache};
//! use std::sync::Arc;
//!
//! let runtime = Runtime::new().with_cache(Arc::new(MemoryCache::new()));
//! ```

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

use crate::dyn_task::AnyOutput;

// ─── CacheKey ─────────────────────────────────────────────────────────────────

/// Lookup key for the task result cache.
///
/// Constructed from the task's stable type name, its params snapshot, and a
/// hash of the serialized input value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// Stable, human-readable type name (from [`crate::registry::SerializableTask`]).
    pub type_name: &'static str,
    /// JSON params snapshot (from [`crate::registry::SerializableTask::params`]).
    pub params: serde_json::Value,
    /// `DefaultHasher` hash of the serialized input bytes.
    pub input_hash: u64,
}

impl CacheKey {
    /// Build a cache key.
    pub fn new(type_name: &'static str, params: serde_json::Value, input_hash: u64) -> Self {
        Self {
            type_name,
            params,
            input_hash,
        }
    }
}

/// Hash the serialized bytes of a `Serialize` value using [`DefaultHasher`].
///
/// Returns `None` if serialization fails. Callers skip caching when `None` is
/// returned.
pub fn hash_input<S: serde::Serialize>(value: &S) -> Option<u64> {
    let bytes = serde_json::to_vec(value).ok()?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Some(hasher.finish())
}

// ─── CachedOutput ─────────────────────────────────────────────────────────────

/// A cloneable wrapper around a type-erased output value.
///
/// `AnyOutput` (`Box<dyn Any + Send>`) is not `Clone`. `CachedOutput` pairs
/// the boxed value with a type-erased clone function so the cache can hand out
/// multiple copies without knowing the concrete type.
///
/// Construct via [`make_cached`], not directly.
/// Type alias for the shared value stored inside a [`CachedOutput`].
pub type SharedValue = Arc<dyn std::any::Any + Send + Sync>;

/// Type alias for the clone function inside a [`CachedOutput`].
type CloneFn = Arc<dyn Fn(&SharedValue) -> AnyOutput + Send + Sync>;

pub struct CachedOutput {
    /// The stored value, behind a shared reference so cache hits can be cloned.
    pub value: SharedValue,
    /// Clone function: produce a fresh `AnyOutput` from the stored Arc.
    clone_fn: CloneFn,
}

impl CachedOutput {
    /// Produce a fresh `AnyOutput` clone.
    pub fn clone_output(&self) -> AnyOutput {
        (self.clone_fn)(&self.value)
    }
}

impl std::fmt::Debug for CachedOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedOutput").finish_non_exhaustive()
    }
}

/// Wrap a concrete output value into a [`CachedOutput`].
///
/// `O` must be `Clone + Send + Sync + 'static`. The runtime calls this for
/// tasks whose output types satisfy those bounds.
pub fn make_cached<O>(value: O) -> CachedOutput
where
    O: Clone + Send + Sync + 'static,
{
    let arc: Arc<dyn std::any::Any + Send + Sync> = Arc::new(value);
    CachedOutput {
        value: arc,
        clone_fn: Arc::new(|any_arc| {
            let typed = any_arc
                .downcast_ref::<O>()
                .expect("CachedOutput clone_fn: type mismatch (internal invariant)");
            Box::new(typed.clone())
        }),
    }
}

// ─── TaskCache trait ──────────────────────────────────────────────────────────

/// Pluggable cache for task results.
///
/// Implementors must be `Send + Sync`. Internal mutation must be handled by
/// the implementor (e.g. via a `Mutex`).
///
/// Only successful outputs are cached; errors are never stored.
pub trait TaskCache: Send + Sync {
    /// Look up a cached output. Returns `None` on miss.
    fn get(&self, key: &CacheKey) -> Option<AnyOutput>;

    /// Insert a successful output into the cache.
    fn insert(&self, key: CacheKey, output: CachedOutput);
}

// ─── NoCache ──────────────────────────────────────────────────────────────────

/// A no-op cache implementation that always misses.
///
/// This is the default — tasks run unconditionally with zero overhead.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoCache;

impl TaskCache for NoCache {
    #[inline]
    fn get(&self, _key: &CacheKey) -> Option<AnyOutput> {
        None
    }

    #[inline]
    fn insert(&self, _key: CacheKey, _output: CachedOutput) {}
}

// ─── MemoryCache ──────────────────────────────────────────────────────────────

/// An in-memory, mutex-guarded cache with no eviction.
///
/// Suitable for single-process use. Entries accumulate until the cache is
/// dropped or cleared explicitly.
#[derive(Default)]
pub struct MemoryCache {
    inner: Mutex<HashMap<CacheKey, CachedOutput>>,
}

impl std::fmt::Debug for MemoryCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.inner.lock().map(|m| m.len()).unwrap_or(0);
        f.debug_struct("MemoryCache").field("len", &len).finish()
    }
}

impl MemoryCache {
    /// Create a new, empty memory cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of entries currently stored.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|m| m.len()).unwrap_or(0)
    }

    /// Returns `true` if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Evict all cached entries.
    pub fn clear(&self) {
        if let Ok(mut m) = self.inner.lock() {
            m.clear();
        }
    }
}

impl TaskCache for MemoryCache {
    fn get(&self, key: &CacheKey) -> Option<AnyOutput> {
        let map = self.inner.lock().ok()?;
        map.get(key).map(|entry| entry.clone_output())
    }

    fn insert(&self, key: CacheKey, output: CachedOutput) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(key, output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_hash_stability() {
        let a = hash_input(&serde_json::json!({"x": 1})).unwrap();
        let b = hash_input(&serde_json::json!({"x": 1})).unwrap();
        assert_eq!(a, b);

        let c = hash_input(&serde_json::json!({"x": 2})).unwrap();
        assert_ne!(a, c);
    }

    #[test]
    fn no_cache_always_misses() {
        let cache = NoCache;
        let key = CacheKey::new("T", serde_json::json!({}), 0);
        assert!(cache.get(&key).is_none());
        cache.insert(key.clone(), make_cached(42u64));
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn memory_cache_hit_and_miss() {
        let cache = MemoryCache::new();
        let key = CacheKey::new("T", serde_json::json!({}), 42);

        assert!(cache.get(&key).is_none());

        cache.insert(key.clone(), make_cached(String::from("hello")));
        assert_eq!(cache.len(), 1);

        let got = cache.get(&key).expect("cache hit");
        // The value comes back as Box<Arc<String>> from clone_fn — downcast to
        // String directly via the inner type.
        let s = got
            .downcast::<String>()
            .expect("downcast to String should succeed");
        assert_eq!(*s, "hello");
    }

    #[test]
    fn memory_cache_clear() {
        let cache = MemoryCache::new();
        let key = CacheKey::new("T", serde_json::json!({}), 0);
        cache.insert(key, make_cached(1u32));
        assert_eq!(cache.len(), 1);
        cache.clear();
        assert!(cache.is_empty());
    }
}
