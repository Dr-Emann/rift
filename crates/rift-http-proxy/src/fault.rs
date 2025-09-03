use crate::config::FaultConfig;
use hyper::{Response, StatusCode};
use http_body_util::Full;
use bytes::Bytes;
use std::time::Duration;
use rand::Rng;

#[derive(Debug, Clone)]
pub enum FaultDecision {
    None,
    Latency(Duration),
    Error { status: u16, body: Option<String> },
}

pub fn decide_fault(config: &Option<FaultConfig>) -> FaultDecision {
    let config = match config {
        Some(c) => c,
        None => return FaultDecision::None,
    };

    if let Some(ref latency) = config.latency {
        let mut rng = rand::thread_rng();
        let ms = rng.gen_range(latency.min_ms..=latency.max_ms);
        return FaultDecision::Latency(Duration::from_millis(ms));
    }

    if let Some(ref error) = config.error {
        return FaultDecision::Error {
            status: error.status,
            body: error.body.clone(),
        };
    }

    FaultDecision::None
}

pub fn create_error_response(status: u16, body: Option<String>) -> Response<Full<Bytes>> {
    let body_bytes = body.unwrap_or_else(|| format!("Error {}", status));
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
        .body(Full::new(Bytes::from(body_bytes)))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LatencyFault, ErrorFault};

    #[test]
    fn test_no_fault() {
        let decision = decide_fault(&None);
        assert!(matches!(decision, FaultDecision::None));
    }

    #[test]
    fn test_error_fault() {
        let config = Some(FaultConfig {
            error: Some(ErrorFault { status: 503, body: Some("Down".into()) }),
            latency: None,
        });
        let decision = decide_fault(&config);
        assert!(matches!(decision, FaultDecision::Error { status: 503, .. }));
    }
}
