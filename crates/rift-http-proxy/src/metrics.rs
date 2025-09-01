use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

lazy_static::lazy_static! {
    pub static ref METRICS: Metrics = Metrics::new();
}

pub struct Metrics {
    requests_total: AtomicU64,
    request_counts: RwLock<HashMap<String, u64>>,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            request_counts: RwLock::new(HashMap::new()),
        }
    }

    pub fn record_request(&self, path: &str) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        let mut counts = self.request_counts.write();
        *counts.entry(path.to_string()).or_insert(0) += 1;
    }

    pub fn collect(&self) -> String {
        let total = self.requests_total.load(Ordering::Relaxed);
        format!("# HELP rift_requests_total Total requests\n# TYPE rift_requests_total counter\nrift_requests_total {}\n", total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_request() {
        let m = Metrics::new();
        m.record_request("/api");
        m.record_request("/api");
        assert!(m.collect().contains("2"));
    }
}
