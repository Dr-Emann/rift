use crate::behaviors::ResponseBehaviors;
use crate::predicate::{BodyMatcher, HeaderMatcher, QueryMatcher};
use crate::recording::ProxyMode;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Protocol supported by Rift for listeners and upstreams
/// Extensible design to support future protocols (TCP, WebSocket, DynamoDB, etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Protocol {
    /// HTTP protocol
    #[default]
    Http,
    /// HTTPS protocol (HTTP over TLS)
    Https,
    /// TCP protocol (for future support)
    #[serde(rename = "tcp")]
    Tcp,
    /// WebSocket protocol (for future support)
    #[serde(rename = "websocket")]
    WebSocket,
    /// DynamoDB protocol (for future support - Mountebank compatibility)
    #[serde(rename = "dynamodb")]
    DynamoDB,
}

impl Protocol {
    /// Check if protocol is currently supported
    pub fn is_supported(&self) -> bool {
        matches!(self, Protocol::Http | Protocol::Https)
    }

    /// Get protocol name as string
    pub fn as_str(&self) -> &'static str {
        match self {
            Protocol::Http => "http",
            Protocol::Https => "https",
            Protocol::Tcp => "tcp",
            Protocol::WebSocket => "websocket",
            Protocol::DynamoDB => "dynamodb",
        }
    }

    /// Parse protocol from URL scheme
    pub fn from_scheme(scheme: &str) -> Result<Self, String> {
        match scheme.to_lowercase().as_str() {
            "http" => Ok(Protocol::Http),
            "https" => Ok(Protocol::Https),
            "tcp" => Ok(Protocol::Tcp),
            "ws" | "websocket" => Ok(Protocol::WebSocket),
            "dynamodb" => Ok(Protocol::DynamoDB),
            _ => Err(format!("Unsupported protocol scheme: {scheme}")),
        }
    }
}

/// Deployment mode for Rift proxy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeploymentMode {
    /// Sidecar mode: single upstream target
    Sidecar,
    /// Reverse proxy mode: multiple upstreams with routing
    ReverseProxy,
}

/// Recording configuration for proxy record/replay (Mountebank-compatible)
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecordingConfig {
    /// Recording mode: proxyOnce, proxyAlways, or proxyTransparent (default)
    #[serde(default)]
    pub mode: ProxyMode,

    /// Capture actual response latency in recorded response (Mountebank addWaitBehavior)
    #[serde(default)]
    pub add_wait_behavior: bool,

    /// Auto-generate stubs from recorded requests (Mountebank predicateGenerators)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub predicate_generators: Vec<PredicateGenerator>,

    /// Persistence configuration for recordings
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistence: Option<RecordingPersistence>,
}

/// Predicate generator for auto-generating stubs from recorded requests
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PredicateGenerator {
    /// Which request fields to match on
    #[serde(default)]
    pub matches: PredicateGeneratorMatches,
}

/// Fields to match on when generating predicates
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PredicateGeneratorMatches {
    /// Include method in predicate
    #[serde(default)]
    pub method: bool,
    /// Include path in predicate
    #[serde(default)]
    pub path: bool,
    /// Include query parameters in predicate
    #[serde(default)]
    pub query: bool,
    /// Include specific headers in predicate
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<String>,
}

/// Persistence configuration for recordings
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingPersistence {
    /// Persistence type: "file" or "redis"
    #[serde(default = "default_persistence_type")]
    pub backend: String,
    /// File path for file-based persistence
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Redis URL for Redis-based persistence
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redis_url: Option<String>,
}

fn default_persistence_type() -> String {
    "file".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Optional, informational only. The config is self-describing and supports
    /// combining features (probabilistic rules, script rules, multi-upstream).
    /// Deprecated: kept for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Deployment mode: "sidecar" or "reverse-proxy"
    /// Recommended: specify explicitly for clarity
    /// If omitted, inferred from upstream/upstreams presence (backward compatible)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<DeploymentMode>,

    pub listen: ListenConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,

    // ===== Deployment Mode Configuration =====
    // Choose exactly ONE deployment mode:
    //
    // SIDECAR MODE:
    //   - Define 'upstream' (single target)
    //   - Do NOT define 'upstreams' or 'routing'
    //   - Traffic: Client -> Rift -> Single Upstream
    //
    // REVERSE PROXY MODE:
    //   - Define 'upstreams' (list of named services)
    //   - Define 'routing' (map requests to upstream names)
    //   - Do NOT define 'upstream'
    //   - Traffic: Client -> Rift -> Multiple Upstreams (routed)
    /// Single upstream target for sidecar mode
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<UpstreamConfig>,

    /// Multiple upstream targets for reverse proxy mode
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upstreams: Vec<Upstream>,

    /// Routing rules for reverse proxy mode (required when 'upstreams' is used)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routing: Vec<Route>,

    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub script_engine: Option<ScriptEngineConfig>,
    #[serde(default)]
    pub flow_state: Option<FlowStateConfig>,
    #[serde(default)]
    pub script_rules: Vec<ScriptRule>,
    #[serde(default)]
    pub connection_pool: ConnectionPoolConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_pool: Option<ScriptPoolConfigFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_cache: Option<DecisionCacheConfigFile>,
    /// Recording configuration for proxy record/replay (Mountebank-compatible)
    #[serde(default)]
    pub recording: RecordingConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConnectionPoolConfig {
    #[serde(default = "default_pool_max_idle_per_host")]
    pub max_idle_per_host: usize,

    #[serde(default = "default_pool_idle_timeout")]
    pub idle_timeout_secs: u64,

    #[serde(default = "default_keepalive_timeout")]
    pub keepalive_timeout_secs: u64,

    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
}

impl Default for ConnectionPoolConfig {
    fn default() -> Self {
        Self {
            max_idle_per_host: default_pool_max_idle_per_host(),
            idle_timeout_secs: default_pool_idle_timeout(),
            keepalive_timeout_secs: default_keepalive_timeout(),
            connect_timeout_secs: default_connect_timeout(),
        }
    }
}

fn default_pool_max_idle_per_host() -> usize {
    100
}
fn default_pool_idle_timeout() -> u64 {
    90
}
fn default_keepalive_timeout() -> u64 {
    60
}
fn default_connect_timeout() -> u64 {
    5
}

/// TLS configuration for HTTPS listener
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TlsConfig {
    /// Path to TLS certificate file (PEM format)
    pub cert_path: String,
    /// Path to TLS private key file (PEM format)
    pub key_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ListenConfig {
    pub port: u16,
    /// Number of worker threads (0 = auto-detect CPU count)
    #[serde(default)]
    pub workers: usize,
    /// Protocol for listener (http or https)
    #[serde(default)]
    pub protocol: Protocol,
    /// TLS configuration (required when protocol is https)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetricsConfig {
    #[serde(default = "default_metrics_port")]
    pub port: u16,
}

fn default_metrics_port() -> u16 {
    9090
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            port: default_metrics_port(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpstreamConfig {
    pub host: String,
    pub port: u16,
    /// Protocol: http or https (default: http)
    /// Note: 'scheme' is deprecated but maintained for backward compatibility
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<Protocol>,
    /// Deprecated: use 'protocol' instead
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    /// Skip TLS certificate verification (for self-signed certs in dev/test)
    #[serde(default)]
    pub tls_skip_verify: bool,
}

impl UpstreamConfig {
    /// Get the protocol, checking both new 'protocol' field and legacy 'scheme' field
    pub fn get_protocol(&self) -> Protocol {
        // Prefer new 'protocol' field
        if let Some(protocol) = self.protocol {
            return protocol;
        }

        // Fall back to legacy 'scheme' field
        if let Some(ref scheme) = self.scheme {
            Protocol::from_scheme(scheme).unwrap_or(Protocol::Http)
        } else {
            Protocol::Http
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub id: String,
    #[serde(rename = "match")]
    pub match_config: MatchConfig,
    pub fault: FaultConfig,
    // Optional: scope fault to specific upstream (v3 multi-upstream mode)
    // If None, applies to all upstreams
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct MatchConfig {
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub path: PathMatch,
    /// Simple header matching (backward compatible)
    #[serde(default)]
    pub headers: Vec<HeaderMatch>,

    // ===== Enhanced Mountebank-compatible predicates =====
    /// Enhanced header matching with operators (contains, startsWith, etc.)
    /// Use this OR headers, not both
    #[serde(
        default,
        rename = "headerPredicates",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub header_predicates: Vec<HeaderMatcher>,

    /// Query parameter matching
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub query: Vec<QueryMatcher>,

    /// Request body matching
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<BodyMatcher>,

    /// Case-sensitive matching (default: true)
    #[serde(default = "default_case_sensitive", rename = "caseSensitive")]
    pub case_sensitive: bool,
}

fn default_case_sensitive() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(untagged)]
pub enum PathMatch {
    #[default]
    Any,
    Exact {
        exact: String,
    },
    Prefix {
        prefix: String,
    },
    Regex {
        regex: String,
    },
    /// Path contains substring (Mountebank-compatible)
    Contains {
        contains: String,
    },
    /// Path ends with suffix (Mountebank-compatible)
    EndsWith {
        #[serde(rename = "endsWith")]
        ends_with: String,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HeaderMatch {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct FaultConfig {
    #[serde(default)]
    pub latency: Option<LatencyFault>,
    #[serde(default)]
    pub error: Option<ErrorFault>,
    /// TCP-level fault (Mountebank-compatible)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcp_fault: Option<TcpFault>,
}

/// TCP-level fault types (Mountebank-compatible)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TcpFault {
    /// Immediately close TCP connection with RST
    ConnectionResetByPeer,
    /// Send random garbage data then close
    RandomDataThenClose,
}

// v2 config types

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScriptEngineConfig {
    #[serde(default = "default_engine_type")]
    pub engine: String, // "rhai" or "lua"
}

fn default_engine_type() -> String {
    "rhai".to_string()
}

impl Default for ScriptEngineConfig {
    fn default() -> Self {
        Self {
            engine: default_engine_type(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FlowStateConfig {
    #[serde(default = "default_backend_type")]
    pub backend: String, // "inmemory", "redis", "valkey"
    #[serde(default = "default_ttl_seconds")]
    pub ttl_seconds: i64,
    #[serde(default)]
    pub redis: Option<RedisConfig>,
}

fn default_backend_type() -> String {
    "inmemory".to_string()
}

fn default_ttl_seconds() -> i64 {
    300
}

impl Default for FlowStateConfig {
    fn default() -> Self {
        Self {
            backend: default_backend_type(),
            ttl_seconds: default_ttl_seconds(),
            redis: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RedisConfig {
    pub url: String,
    #[serde(default = "default_redis_pool_size")]
    pub pool_size: usize,
    #[serde(default = "default_redis_key_prefix")]
    pub key_prefix: String,
}

fn default_redis_pool_size() -> usize {
    10
}

fn default_redis_key_prefix() -> String {
    "rift:".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScriptRule {
    pub id: String,
    pub script: String, // inline script or path to script file
    #[serde(default, rename = "match")]
    pub match_config: MatchConfig,
    // Optional: scope fault to specific upstream (v3 multi-upstream mode)
    // If None, applies to all upstreams
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LatencyFault {
    pub probability: f64,
    pub min_ms: u64,
    pub max_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ErrorFault {
    pub probability: f64,
    pub status: u16,
    #[serde(default)]
    pub body: String,
    /// Optional headers to include in error response (can be overridden by script headers)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub headers: std::collections::HashMap<String, String>,
    /// Mountebank-compatible response behaviors (wait, repeat, copy, lookup)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behaviors: Option<ResponseBehaviors>,
}

// Multi-upstream types (v3)

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Upstream {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub health_check: Option<HealthCheckConfig>,
    /// Skip TLS certificate verification (for self-signed certs in dev/test)
    #[serde(default)]
    pub tls_skip_verify: bool,
}

impl Upstream {
    /// Parse and extract protocol from URL
    /// Returns the protocol or an error if URL is invalid or protocol is unsupported
    pub fn get_protocol(&self) -> Result<Protocol, String> {
        // Parse URL to extract scheme
        let url_parts: Vec<&str> = self.url.splitn(2, "://").collect();
        if url_parts.len() != 2 {
            return Err(format!("Invalid URL format (missing scheme): {}", self.url));
        }

        Protocol::from_scheme(url_parts[0])
    }

    /// Validate that the upstream configuration is valid
    pub fn validate(&self) -> Result<(), String> {
        // Check protocol is valid and supported
        let protocol = self.get_protocol()?;
        if !protocol.is_supported() {
            return Err(format!(
                "Unsupported protocol '{}' for upstream '{}'. Currently supported: http, https",
                protocol.as_str(),
                self.name
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HealthCheckConfig {
    #[serde(default = "default_health_path")]
    pub path: String,
    #[serde(default = "default_health_interval")]
    pub interval_seconds: u64,
    #[serde(default = "default_health_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "default_health_unhealthy_threshold")]
    pub unhealthy_threshold: u32,
    #[serde(default = "default_health_healthy_threshold")]
    pub healthy_threshold: u32,
}

fn default_health_path() -> String {
    "/health".to_string()
}

fn default_health_interval() -> u64 {
    30
}

fn default_health_timeout() -> u64 {
    5
}

fn default_health_unhealthy_threshold() -> u32 {
    3
}

fn default_health_healthy_threshold() -> u32 {
    2
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            path: default_health_path(),
            interval_seconds: default_health_interval(),
            timeout_seconds: default_health_timeout(),
            unhealthy_threshold: default_health_unhealthy_threshold(),
            healthy_threshold: default_health_healthy_threshold(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Route {
    pub name: String,
    #[serde(rename = "match")]
    pub match_config: RouteMatch,
    pub upstream: String, // upstream name
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RouteMatch {
    #[serde(default)]
    pub host: Option<HostMatch>,
    #[serde(default)]
    pub path_prefix: Option<String>,
    #[serde(default)]
    pub path_exact: Option<String>,
    #[serde(default)]
    pub path_regex: Option<String>,
    #[serde(default)]
    pub headers: Vec<HeaderMatch>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum HostMatch {
    Exact(String),
    Wildcard { wildcard: String },
}

/// Script pool configuration (M9 Phase 4 optimization)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScriptPoolConfigFile {
    /// Number of worker threads (0 = auto-detect: num_cpus/2, min 2, max 16)
    #[serde(default = "default_script_pool_workers")]
    pub workers: usize,
    /// Maximum queue size for pending script executions
    #[serde(default = "default_script_pool_queue_size")]
    pub queue_size: usize,
    /// Timeout in milliseconds for script execution
    #[serde(default = "default_script_pool_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_script_pool_workers() -> usize {
    0
} // 0 = auto-detect
fn default_script_pool_queue_size() -> usize {
    1000
}
fn default_script_pool_timeout_ms() -> u64 {
    5000
}

impl Default for ScriptPoolConfigFile {
    fn default() -> Self {
        Self {
            workers: default_script_pool_workers(),
            queue_size: default_script_pool_queue_size(),
            timeout_ms: default_script_pool_timeout_ms(),
        }
    }
}

/// Decision cache configuration (M9 Phase 4 optimization)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DecisionCacheConfigFile {
    /// Enable decision caching
    #[serde(default = "default_decision_cache_enabled")]
    pub enabled: bool,
    /// Maximum number of cache entries (LRU eviction when exceeded)
    #[serde(default = "default_decision_cache_max_size")]
    pub max_size: usize,
    /// TTL for cache entries in seconds (0 = no expiration)
    #[serde(default = "default_decision_cache_ttl_seconds")]
    pub ttl_seconds: u64,
}

fn default_decision_cache_enabled() -> bool {
    true
}
fn default_decision_cache_max_size() -> usize {
    10000
}
fn default_decision_cache_ttl_seconds() -> u64 {
    300
}

impl Default for DecisionCacheConfigFile {
    fn default() -> Self {
        Self {
            enabled: default_decision_cache_enabled(),
            max_size: default_decision_cache_max_size(),
            ttl_seconds: default_decision_cache_ttl_seconds(),
        }
    }
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, anyhow::Error> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = serde_yaml::from_str(&contents)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        // Validate listener configuration
        if self.listen.protocol == Protocol::Https && self.listen.tls.is_none() {
            anyhow::bail!(
                "TLS configuration is required when listener protocol is 'https'. \
                 Please provide 'listen.tls.cert_path' and 'listen.tls.key_path'"
            );
        }

        // Validate listener protocol is supported
        if !self.listen.protocol.is_supported() {
            anyhow::bail!(
                "Unsupported listener protocol: '{}'. Currently supported: http, https",
                self.listen.protocol.as_str()
            );
        }

        // Validate upstream configuration (sidecar mode)
        if let Some(ref upstream) = self.upstream {
            let protocol = upstream.get_protocol();
            if !protocol.is_supported() {
                anyhow::bail!(
                    "Unsupported upstream protocol: '{}'. Currently supported: http, https",
                    protocol.as_str()
                );
            }
        }

        // Validate all upstreams (reverse proxy mode)
        for upstream in &self.upstreams {
            upstream.validate().map_err(|e| anyhow::anyhow!(e))?;
        }

        // Validate script rules if present
        self.validate_script_rules()?;

        Ok(())
    }

    /// Validate all script rules based on the configured script engine
    fn validate_script_rules(&self) -> Result<(), anyhow::Error> {
        if self.script_rules.is_empty() {
            return Ok(());
        }

        let engine_type = self
            .script_engine
            .as_ref()
            .map(|cfg| cfg.engine.as_str())
            .unwrap_or("rhai");

        for script_rule in &self.script_rules {
            match engine_type {
                "rhai" => {
                    use crate::scripting::RhaiValidator;
                    RhaiValidator::new()
                        .validate(&script_rule.script)
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Invalid Rhai script in rule '{}': {}",
                                script_rule.id,
                                e
                            )
                        })?;
                }
                #[cfg(feature = "lua")]
                "lua" => {
                    use crate::scripting::LuaValidator;
                    LuaValidator::new()
                        .validate(&script_rule.script)
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Invalid Lua script in rule '{}': {}",
                                script_rule.id,
                                e
                            )
                        })?;
                }
                #[cfg(not(feature = "lua"))]
                "lua" => {
                    anyhow::bail!("Lua engine specified but 'lua' feature is not enabled");
                }
                #[cfg(feature = "javascript")]
                "javascript" | "js" => {
                    use crate::scripting::JsValidator;
                    JsValidator::validate(&script_rule.script).map_err(|e| {
                        anyhow::anyhow!(
                            "Invalid JavaScript script in rule '{}': {}",
                            script_rule.id,
                            e
                        )
                    })?;
                }
                #[cfg(not(feature = "javascript"))]
                "javascript" | "js" => {
                    anyhow::bail!(
                        "JavaScript engine specified but 'javascript' feature is not enabled"
                    );
                }
                other => {
                    anyhow::bail!("Unknown script engine type: '{other}'");
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let yaml = r#"
version: v1
listen:
  port: 8080
metrics:
  port: 9090
upstream:
  host: 127.0.0.1
  port: 8000
rules:
  - id: "test-latency"
    match:
      methods: ["POST"]
      path:
        prefix: "/api"
    fault:
      latency:
        probability: 0.1
        min_ms: 100
        max_ms: 500
  - id: "test-error"
    match:
      methods: ["GET"]
      path:
        exact: "/fail"
    fault:
      error:
        probability: 0.5
        status: 502
        body: '{"error": "injected"}'
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.version, Some("v1".to_string()));
        assert_eq!(config.listen.port, 8080);
        assert_eq!(config.upstream.as_ref().unwrap().port, 8000);
        assert_eq!(config.rules.len(), 2);
        assert_eq!(config.rules[0].id, "test-latency");
        assert!(config.rules[0].fault.latency.is_some());
        assert_eq!(config.rules[1].id, "test-error");
        assert!(config.rules[1].fault.error.is_some());
    }

    #[test]
    fn test_parse_v2_config() {
        let yaml = r#"
version: v2
listen:
  port: 8080
upstream:
  host: 127.0.0.1
  port: 8000
script_engine:
  engine: rhai
flow_state:
  backend: inmemory
  ttl_seconds: 300
script_rules:
  - id: "progressive-failure"
    script: |
      fn should_inject_fault(request, flow_store) {
        let flow_id = request.headers["x-flow-id"];
        let attempts = flow_store.increment(flow_id, "attempts");
        if attempts <= 2 {
          return #{ inject: true, fault: "error", status: 503, body: "Retry" };
        }
        return #{ inject: false };
      }
      should_inject_fault(request, flow_store)
    match:
      methods: ["POST"]
      path:
        prefix: "/api"
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.version, Some("v2".to_string()));
        assert!(config.script_engine.is_some());
        assert_eq!(config.script_engine.unwrap().engine, "rhai");
        assert!(config.flow_state.is_some());
        assert_eq!(config.flow_state.as_ref().unwrap().backend, "inmemory");
        assert_eq!(config.flow_state.as_ref().unwrap().ttl_seconds, 300);
        assert_eq!(config.script_rules.len(), 1);
        assert_eq!(config.script_rules[0].id, "progressive-failure");
        assert!(config.script_rules[0]
            .script
            .contains("should_inject_fault"));
    }

    #[test]
    fn test_parse_v3_multi_upstream_config() {
        let yaml = r#"
version: v3
listen:
  port: 8080
upstreams:
  - name: service-a
    url: "http://service-a:8000"
    health_check:
      path: "/health"
      interval_seconds: 30
  - name: service-b
    url: "http://service-b:8001"
routing:
  - name: "route-to-a"
    match:
      path_prefix: "/api/users"
    upstream: service-a
  - name: "route-to-b"
    match:
      path_prefix: "/api/orders"
      headers:
        - name: "x-version"
          value: "v2"
    upstream: service-b
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.version, Some("v3".to_string()));

        // Verify multi-upstream mode
        assert!(config.upstream.is_none());
        assert_eq!(config.upstreams.len(), 2);
        assert_eq!(config.upstreams[0].name, "service-a");
        assert_eq!(config.upstreams[0].url, "http://service-a:8000");
        assert!(config.upstreams[0].health_check.is_some());
        assert_eq!(config.upstreams[1].name, "service-b");

        // Verify routing
        assert_eq!(config.routing.len(), 2);
        assert_eq!(config.routing[0].name, "route-to-a");
        assert_eq!(config.routing[0].upstream, "service-a");
        assert_eq!(
            config.routing[0].match_config.path_prefix,
            Some("/api/users".to_string())
        );

        assert_eq!(config.routing[1].name, "route-to-b");
        assert_eq!(config.routing[1].upstream, "service-b");
        assert_eq!(config.routing[1].match_config.headers.len(), 1);
        assert_eq!(config.routing[1].match_config.headers[0].name, "x-version");
    }

    #[test]
    fn test_parse_error_fault_with_headers() {
        let yaml = r#"
version: v1
listen:
  port: 8080
upstream:
  host: 127.0.0.1
  port: 8000
rules:
  - id: "error-with-headers"
    match:
      methods: ["GET"]
      path:
        prefix: "/api"
    fault:
      error:
        probability: 1.0
        status: 502
        body: '{"error":"Service unavailable"}'
        headers:
          Server: "openresty"
          X-Content-Type-Options: "nosniff"
          Cache-Control: "no-cache, no-store, max-age=0, must-revalidate"
          x-apigw-key: "CapiOne-IT-INT"
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].id, "error-with-headers");

        let error_fault = config.rules[0].fault.error.as_ref().unwrap();
        assert_eq!(error_fault.status, 502);
        assert_eq!(error_fault.headers.len(), 4);
        assert_eq!(
            error_fault.headers.get("Server"),
            Some(&"openresty".to_string())
        );
        assert_eq!(
            error_fault.headers.get("X-Content-Type-Options"),
            Some(&"nosniff".to_string())
        );
        assert_eq!(
            error_fault.headers.get("x-apigw-key"),
            Some(&"CapiOne-IT-INT".to_string())
        );
    }

    #[test]
    fn test_parse_per_upstream_fault_rules() {
        let yaml = r#"
version: v3
listen:
  port: 8080
upstreams:
  - name: service-a
    url: "http://service-a:8000"
  - name: service-b
    url: "http://service-b:8001"
routing:
  - name: "route-a"
    match:
      path_prefix: "/api/a"
    upstream: service-a
  - name: "route-b"
    match:
      path_prefix: "/api/b"
    upstream: service-b
rules:
  # Global rule (applies to all upstreams)
  - id: "global-latency"
    match:
      methods: ["GET"]
    fault:
      latency:
        probability: 0.1
        min_ms: 100
        max_ms: 200
  # Service-specific rule (only applies to service-a)
  - id: "service-a-error"
    upstream: service-a
    match:
      methods: ["POST"]
    fault:
      error:
        probability: 0.5
        status: 503
        body: "Service A unavailable"
  # Another service-specific rule (only applies to service-b)
  - id: "service-b-latency"
    upstream: service-b
    match:
      path:
        prefix: "/api/b"
    fault:
      latency:
        probability: 0.8
        min_ms: 500
        max_ms: 1000
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.version, Some("v3".to_string()));

        // Verify rules with upstream filters
        assert_eq!(config.rules.len(), 3);

        // Global rule - no upstream filter
        assert_eq!(config.rules[0].id, "global-latency");
        assert!(config.rules[0].upstream.is_none());

        // Service-specific rules
        assert_eq!(config.rules[1].id, "service-a-error");
        assert_eq!(config.rules[1].upstream.as_ref().unwrap(), "service-a");
        assert!(config.rules[1].fault.error.is_some());

        assert_eq!(config.rules[2].id, "service-b-latency");
        assert_eq!(config.rules[2].upstream.as_ref().unwrap(), "service-b");
        assert!(config.rules[2].fault.latency.is_some());
    }

    #[test]
    fn test_parse_mountebank_behaviors() {
        let yaml = r#"
listen:
  port: 8080
upstream:
  host: localhost
  port: 9000
rules:
  - id: "behavior-wait-fixed"
    match:
      path:
        prefix: "/wait-fixed"
    fault:
      error:
        probability: 1.0
        status: 200
        body: '{"result": "delayed"}'
        behaviors:
          wait: 100
  - id: "behavior-wait-range"
    match:
      path:
        prefix: "/wait-range"
    fault:
      error:
        probability: 1.0
        status: 200
        body: '{"result": "delayed-range"}'
        behaviors:
          wait:
            min: 50
            max: 150
  - id: "tcp-reset"
    match:
      path:
        prefix: "/tcp-reset"
    fault:
      tcp_fault: CONNECTION_RESET_BY_PEER
  - id: "tcp-random"
    match:
      path:
        prefix: "/tcp-random"
    fault:
      tcp_fault: RANDOM_DATA_THEN_CLOSE
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.rules.len(), 4);

        // Test wait behavior - fixed
        let rule1 = &config.rules[0];
        assert_eq!(rule1.id, "behavior-wait-fixed");
        let error1 = rule1.fault.error.as_ref().unwrap();
        assert!(error1.behaviors.is_some());
        let behaviors1 = error1.behaviors.as_ref().unwrap();
        assert!(behaviors1.wait.is_some());

        // Test wait behavior - range
        let rule2 = &config.rules[1];
        assert_eq!(rule2.id, "behavior-wait-range");
        let error2 = rule2.fault.error.as_ref().unwrap();
        assert!(error2.behaviors.is_some());
        let behaviors2 = error2.behaviors.as_ref().unwrap();
        assert!(behaviors2.wait.is_some());

        // Test TCP fault - connection reset
        let rule3 = &config.rules[2];
        assert_eq!(rule3.id, "tcp-reset");
        assert!(rule3.fault.tcp_fault.is_some());
        assert_eq!(
            rule3.fault.tcp_fault.unwrap(),
            TcpFault::ConnectionResetByPeer
        );

        // Test TCP fault - random data
        let rule4 = &config.rules[3];
        assert_eq!(rule4.id, "tcp-random");
        assert!(rule4.fault.tcp_fault.is_some());
        assert_eq!(
            rule4.fault.tcp_fault.unwrap(),
            TcpFault::RandomDataThenClose
        );
    }

    #[test]
    fn test_parse_recording_config_proxy_once() {
        let yaml = r#"
listen:
  port: 8080
upstream:
  host: 127.0.0.1
  port: 8000
recording:
  mode: proxyOnce
rules: []
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.recording.mode, ProxyMode::ProxyOnce);
    }

    #[test]
    fn test_parse_recording_config_proxy_always() {
        let yaml = r#"
listen:
  port: 8080
upstream:
  host: 127.0.0.1
  port: 8000
recording:
  mode: proxyAlways
rules: []
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.recording.mode, ProxyMode::ProxyAlways);
    }

    #[test]
    fn test_parse_recording_config_default_transparent() {
        let yaml = r#"
listen:
  port: 8080
upstream:
  host: 127.0.0.1
  port: 8000
rules: []
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        // Default should be proxyTransparent
        assert_eq!(config.recording.mode, ProxyMode::ProxyTransparent);
    }

    #[test]
    fn test_parse_recording_config_explicit_transparent() {
        let yaml = r#"
listen:
  port: 8080
upstream:
  host: 127.0.0.1
  port: 8000
recording:
  mode: proxyTransparent
rules: []
"#;

        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.recording.mode, ProxyMode::ProxyTransparent);
    }
}
