use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub listen: ListenConfig,
    #[serde(default)]
    pub rules: Vec<Rule>,
    pub upstream: Option<UpstreamConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ListenConfig {
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
}

fn default_host() -> String { "0.0.0.0".to_string() }

#[derive(Debug, Deserialize, Clone)]
pub struct UpstreamConfig {
    pub url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Rule {
    pub id: Option<String>,
    #[serde(rename = "match")]
    pub match_config: Option<MatchConfig>,
    pub fault: Option<FaultConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MatchConfig {
    pub path: Option<String>,
    pub method: Option<String>,
    pub headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FaultConfig {
    pub error: Option<ErrorFault>,
    pub latency: Option<LatencyFault>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ErrorFault {
    pub status: u16,
    pub body: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LatencyFault {
    pub min_ms: u64,
    pub max_ms: u64,
}

impl Config {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        Ok(serde_yaml::from_str(&content)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let yaml = r#"
listen:
  port: 8080
rules: []
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.listen.port, 8080);
    }

    #[test]
    fn test_parse_rule() {
        let yaml = r#"
listen:
  port: 8080
rules:
  - id: test
    match:
      path: /api
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.rules.len(), 1);
    }
}
