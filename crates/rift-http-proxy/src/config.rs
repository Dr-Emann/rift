use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub listen: ListenConfig,
}

#[derive(Debug, Deserialize)]
pub struct ListenConfig {
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
}

fn default_host() -> String { "0.0.0.0".to_string() }

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
        let yaml = "listen:\n  port: 8080";
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.listen.port, 8080);
    }
}
