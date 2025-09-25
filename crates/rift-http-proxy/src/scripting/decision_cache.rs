use crate::scripting::FaultDecision;
use anyhow::Result;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tracing::{debug, trace};

/// Configuration for the decision cache
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct DecisionCacheConfig {
    /// Enable decision caching
    #[allow(dead_code)]
    pub enabled: bool,
    /// Maximum number of cache entries (LRU eviction when exceeded)
    #[allow(dead_code)]
    pub max_size: usize,
    /// TTL for cache entries in seconds (0 = no expiration)
    #[allow(dead_code)]
    pub ttl_seconds: u64,
}

impl Default for DecisionCacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_size: 10000,
            ttl_seconds: 300, // 5 minutes
        }
    }
}

/// Cache key derived from request properties
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct CacheKey {
    /// Request method
    method: String,
    /// Request path
    path: String,
    /// Sorted header keys and values (for deterministic hashing)
    headers: Vec<(String, String)>,
    /// Body hash (to avoid storing large bodies)
    body_hash: u64,
    /// Rule ID
    rule_id: String,
}

impl CacheKey {
    /// Create a new cache key from request properties
    #[allow(dead_code)]
    pub fn new(
        method: String,
        path: String,
        mut headers: Vec<(String, String)>,
        body: &serde_json::Value,
        rule_id: String,
    ) -> Self {
        // Sort headers for deterministic key generation
        headers.sort_by(|a, b| a.0.cmp(&b.0));

        // Hash the body to avoid storing large payloads
        let body_hash = Self::hash_json(body);

        Self {
            method,
            path,
            headers,
            body_hash,
            rule_id,
        }
    }

    /// Hash a JSON value for cache key generation
    #[allow(dead_code)]
    fn hash_json(value: &serde_json::Value) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        // Use canonical JSON string for consistent hashing
        let json_str = serde_json::to_string(value).unwrap_or_default();
        json_str.hash(&mut hasher);
        hasher.finish()
    }
}

/// Cache entry with TTL tracking
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct CacheEntry {
    decision: FaultDecision,
    created_at: Instant,
    last_accessed: Instant,
    access_count: u64,
}

impl CacheEntry {
    #[allow(dead_code)]
    fn new(decision: FaultDecision) -> Self {
        let now = Instant::now();
        Self {
            decision,
            created_at: now,
            last_accessed: now,
            access_count: 0,
        }
    }

    #[allow(dead_code)]
    fn is_expired(&self, ttl: Duration) -> bool {
        if ttl.is_zero() {
            return false; // No expiration
        }
        self.created_at.elapsed() > ttl
    }

    #[allow(dead_code)]
    fn touch(&mut self) {
        self.last_accessed = Instant::now();
        self.access_count += 1;
    }
}

/// Metrics for cache performance
#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
pub struct CacheMetrics {
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
    pub expirations: u64,
    pub size: usize,
}

impl CacheMetrics {
    #[allow(dead_code)]
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// Decision cache for memoizing script execution results
#[allow(dead_code)]
pub struct DecisionCache {
    config: DecisionCacheConfig,
    cache: Arc<RwLock<HashMap<CacheKey, CacheEntry>>>,
    metrics: Arc<RwLock<CacheMetrics>>,
}

impl DecisionCache {
    /// Create a new decision cache
    #[allow(dead_code)]
    pub fn new(config: DecisionCacheConfig) -> Self {
        debug!(
            "Creating decision cache: enabled={}, max_size={}, ttl={}s",
            config.enabled, config.max_size, config.ttl_seconds
        );

        Self {
            config,
            cache: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(RwLock::new(CacheMetrics::default())),
        }
    }

    /// Get a decision from cache if available and not expired
    #[allow(dead_code)]
    pub fn get(&self, key: &CacheKey) -> Option<FaultDecision> {
        if !self.config.enabled {
            return None;
        }

        let mut cache = self.cache.write().unwrap();
        let ttl = Duration::from_secs(self.config.ttl_seconds);

        if let Some(entry) = cache.get_mut(key) {
            // Check if entry is expired
            if entry.is_expired(ttl) {
                trace!("Cache entry expired for key: {:?}", key);
                cache.remove(key);

                // Update metrics
                let mut metrics = self.metrics.write().unwrap();
                metrics.misses += 1;
                metrics.expirations += 1;
                metrics.size = cache.len();

                return None;
            }

            // Entry is valid, update access time and return
            entry.touch();
            trace!(
                "Cache hit for key: {:?} (access_count: {})",
                key,
                entry.access_count
            );

            // Update metrics
            let mut metrics = self.metrics.write().unwrap();
            metrics.hits += 1;

            return Some(entry.decision.clone());
        }

        // Cache miss
        trace!("Cache miss for key: {:?}", key);
        let mut metrics = self.metrics.write().unwrap();
        metrics.misses += 1;

        None
    }

    /// Insert a decision into the cache
    #[allow(dead_code)]
    pub fn insert(&self, key: CacheKey, decision: FaultDecision) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        let mut cache = self.cache.write().unwrap();

        // Check if we need to evict entries
        if cache.len() >= self.config.max_size && !cache.contains_key(&key) {
            self.evict_lru(&mut cache);
        }

        // Insert new entry
        cache.insert(key.clone(), CacheEntry::new(decision));
        trace!("Cache insert for key: {:?}", key);

        // Update metrics
        let mut metrics = self.metrics.write().unwrap();
        metrics.inserts += 1;
        metrics.size = cache.len();

        Ok(())
    }

    /// Evict the least recently used entry
    #[allow(dead_code)]
    fn evict_lru(&self, cache: &mut HashMap<CacheKey, CacheEntry>) {
        // Find entry with oldest last_accessed time
        if let Some((key_to_evict, _)) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_accessed)
            .map(|(k, v)| (k.clone(), v.clone()))
        {
            cache.remove(&key_to_evict);
            trace!("Evicted LRU entry: {:?}", key_to_evict);

            // Update metrics
            let mut metrics = self.metrics.write().unwrap();
            metrics.evictions += 1;
        }
    }

    /// Clear all cache entries
    #[allow(dead_code)]
    pub fn clear(&self) {
        let mut cache = self.cache.write().unwrap();
        cache.clear();

        let mut metrics = self.metrics.write().unwrap();
        metrics.size = 0;

        debug!("Cache cleared");
    }

    /// Get current cache metrics
    #[allow(dead_code)]
    pub fn metrics(&self) -> CacheMetrics {
        self.metrics.read().unwrap().clone()
    }

    /// Remove expired entries (can be called periodically)
    #[allow(dead_code)]
    pub fn cleanup_expired(&self) {
        if !self.config.enabled || self.config.ttl_seconds == 0 {
            return;
        }

        let mut cache = self.cache.write().unwrap();
        let ttl = Duration::from_secs(self.config.ttl_seconds);

        let expired_keys: Vec<CacheKey> = cache
            .iter()
            .filter(|(_, entry)| entry.is_expired(ttl))
            .map(|(k, _)| k.clone())
            .collect();

        let count = expired_keys.len();
        for key in expired_keys {
            cache.remove(&key);
        }

        if count > 0 {
            debug!("Cleaned up {} expired cache entries", count);

            let mut metrics = self.metrics.write().unwrap();
            metrics.expirations += count as u64;
            metrics.size = cache.len();
        }
    }

    /// Get cache size
    #[allow(dead_code)]
    pub fn size(&self) -> usize {
        self.cache.read().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::thread;

    #[test]
    fn test_cache_key_creation() {
        let headers = vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("x-request-id".to_string(), "123".to_string()),
        ];

        let key1 = CacheKey::new(
            "GET".to_string(),
            "/api/test".to_string(),
            headers.clone(),
            &json!({"foo": "bar"}),
            "rule1".to_string(),
        );

        let key2 = CacheKey::new(
            "GET".to_string(),
            "/api/test".to_string(),
            headers.clone(),
            &json!({"foo": "bar"}),
            "rule1".to_string(),
        );

        // Same inputs should produce equal keys
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_key_different_order_headers() {
        let headers1 = vec![
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string()),
        ];

        let headers2 = vec![
            ("b".to_string(), "2".to_string()),
            ("a".to_string(), "1".to_string()),
        ];

        let key1 = CacheKey::new(
            "GET".to_string(),
            "/api/test".to_string(),
            headers1,
            &json!({}),
            "rule1".to_string(),
        );

        let key2 = CacheKey::new(
            "GET".to_string(),
            "/api/test".to_string(),
            headers2,
            &json!({}),
            "rule1".to_string(),
        );

        // Headers in different order should produce same key
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_basic_operations() {
        let config = DecisionCacheConfig {
            enabled: true,
            max_size: 100,
            ttl_seconds: 0, // No expiration for this test
        };

        let cache = DecisionCache::new(config);

        let key = CacheKey::new(
            "GET".to_string(),
            "/api/test".to_string(),
            vec![],
            &json!({}),
            "rule1".to_string(),
        );

        // Cache miss
        assert!(cache.get(&key).is_none());

        // Insert
        let decision = FaultDecision::Latency {
            duration_ms: 100,
            rule_id: "rule1".to_string(),
        };
        cache.insert(key.clone(), decision.clone()).unwrap();

        // Cache hit
        let cached = cache.get(&key).unwrap();
        match cached {
            FaultDecision::Latency { duration_ms, .. } => {
                assert_eq!(duration_ms, 100);
            }
            _ => panic!("Expected Latency decision"),
        }

        // Verify metrics
        let metrics = cache.metrics();
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.misses, 1);
        assert_eq!(metrics.inserts, 1);
        assert_eq!(metrics.size, 1);
    }

    #[test]
    fn test_cache_expiration() {
        let config = DecisionCacheConfig {
            enabled: true,
            max_size: 100,
            ttl_seconds: 1, // 1 second TTL
        };

        let cache = DecisionCache::new(config);

        let key = CacheKey::new(
            "GET".to_string(),
            "/api/test".to_string(),
            vec![],
            &json!({}),
            "rule1".to_string(),
        );

        let decision = FaultDecision::None;
        cache.insert(key.clone(), decision).unwrap();

        // Should be cached
        assert!(cache.get(&key).is_some());

        // Wait for expiration
        thread::sleep(Duration::from_secs(2));

        // Should be expired
        assert!(cache.get(&key).is_none());

        // Verify expiration metric
        let metrics = cache.metrics();
        assert_eq!(metrics.expirations, 1);
    }

    #[test]
    fn test_cache_lru_eviction() {
        let config = DecisionCacheConfig {
            enabled: true,
            max_size: 3,
            ttl_seconds: 0,
        };

        let cache = DecisionCache::new(config);

        // Insert 3 entries
        for i in 0..3 {
            let key = CacheKey::new(
                "GET".to_string(),
                format!("/api/test{i}"),
                vec![],
                &json!({}),
                format!("rule{i}"),
            );
            cache.insert(key, FaultDecision::None).unwrap();
        }

        assert_eq!(cache.size(), 3);

        // Access key 1 and 2 to make key 0 the LRU
        let key1 = CacheKey::new(
            "GET".to_string(),
            "/api/test1".to_string(),
            vec![],
            &json!({}),
            "rule1".to_string(),
        );
        cache.get(&key1);

        let key2 = CacheKey::new(
            "GET".to_string(),
            "/api/test2".to_string(),
            vec![],
            &json!({}),
            "rule2".to_string(),
        );
        cache.get(&key2);

        // Insert 4th entry - should evict key 0 (LRU)
        let key3 = CacheKey::new(
            "GET".to_string(),
            "/api/test3".to_string(),
            vec![],
            &json!({}),
            "rule3".to_string(),
        );
        cache.insert(key3, FaultDecision::None).unwrap();

        assert_eq!(cache.size(), 3);

        // Key 0 should be evicted
        let key0 = CacheKey::new(
            "GET".to_string(),
            "/api/test0".to_string(),
            vec![],
            &json!({}),
            "rule0".to_string(),
        );
        assert!(cache.get(&key0).is_none());

        // Keys 1, 2, 3 should still be present
        assert!(cache.get(&key1).is_some());
        assert!(cache.get(&key2).is_some());

        let metrics = cache.metrics();
        assert_eq!(metrics.evictions, 1);
    }

    #[test]
    fn test_cache_disabled() {
        let config = DecisionCacheConfig {
            enabled: false,
            max_size: 100,
            ttl_seconds: 0,
        };

        let cache = DecisionCache::new(config);

        let key = CacheKey::new(
            "GET".to_string(),
            "/api/test".to_string(),
            vec![],
            &json!({}),
            "rule1".to_string(),
        );

        let decision = FaultDecision::None;
        cache.insert(key.clone(), decision).unwrap();

        // Should always return None when disabled
        assert!(cache.get(&key).is_none());
        assert_eq!(cache.size(), 0);
    }

    #[test]
    fn test_cache_clear() {
        let config = DecisionCacheConfig::default();
        let cache = DecisionCache::new(config);

        // Insert multiple entries
        for i in 0..5 {
            let key = CacheKey::new(
                "GET".to_string(),
                format!("/api/test{i}"),
                vec![],
                &json!({}),
                format!("rule{i}"),
            );
            cache.insert(key, FaultDecision::None).unwrap();
        }

        assert_eq!(cache.size(), 5);

        cache.clear();
        assert_eq!(cache.size(), 0);
    }

    #[test]
    fn test_cache_hit_rate() {
        let config = DecisionCacheConfig::default();
        let cache = DecisionCache::new(config);

        let key = CacheKey::new(
            "GET".to_string(),
            "/api/test".to_string(),
            vec![],
            &json!({}),
            "rule1".to_string(),
        );

        // 1 miss
        cache.get(&key);

        cache.insert(key.clone(), FaultDecision::None).unwrap();

        // 3 hits
        cache.get(&key);
        cache.get(&key);
        cache.get(&key);

        let metrics = cache.metrics();
        assert_eq!(metrics.hits, 3);
        assert_eq!(metrics.misses, 1);
        assert_eq!(metrics.hit_rate(), 0.75); // 3 / (3 + 1)
    }

    #[test]
    fn test_cache_cleanup_expired() {
        let config = DecisionCacheConfig {
            enabled: true,
            max_size: 100,
            ttl_seconds: 1,
        };

        let cache = DecisionCache::new(config);

        // Insert entries
        for i in 0..5 {
            let key = CacheKey::new(
                "GET".to_string(),
                format!("/api/test{i}"),
                vec![],
                &json!({}),
                format!("rule{i}"),
            );
            cache.insert(key, FaultDecision::None).unwrap();
        }

        assert_eq!(cache.size(), 5);

        // Wait for expiration
        thread::sleep(Duration::from_secs(2));

        // Cleanup
        cache.cleanup_expired();

        assert_eq!(cache.size(), 0);

        let metrics = cache.metrics();
        assert_eq!(metrics.expirations, 5);
    }
}
