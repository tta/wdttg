use serde::Deserialize;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Config parse error: {0}")]
    Parse(String),
}

pub type Result<T> = std::result::Result<T, ConfigError>;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub db: Option<String>,
    pub vault: Option<String>,
}

impl Config {
    fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("wdttg")
            .join("wdttg.json")
    }

    fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("wdttg")
    }

    pub fn load() -> Result<Self> {
        let config_path = Self::config_path();

        if !config_path.exists() {
            return Ok(Config::default());
        }

        let content = std::fs::read_to_string(&config_path)?;
        let config: Config =
            json5::from_str(&content).map_err(|e| ConfigError::Parse(e.to_string()))?;
        Ok(config)
    }

    pub fn apply(&self) {
        if let Some(ref db) = self.db {
            if std::env::var("WDTTG_DB").is_err() {
                std::env::set_var("WDTTG_DB", expand_tilde(db));
            }
        }
        if let Some(ref vault) = self.vault {
            if std::env::var("WDTTG_VAULT").is_err() {
                std::env::set_var("WDTTG_VAULT", expand_tilde(vault));
            }
        }
    }
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        dirs::home_dir()
            .map(|h| h.join(&path[2..]))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string())
    } else {
        path.to_string()
    }
}

pub fn init() -> Result<()> {
    let config_dir = Config::config_dir();

    if !config_dir.exists() {
        std::fs::create_dir_all(&config_dir)?;
    }

    let config_path = Config::config_path();
    if !config_path.exists() {
        std::fs::write(&config_path, "{}\n")?;
    }

    let config = Config::load()?;
    config.apply();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_tilde_with_home() {
        let home = dirs::home_dir().expect("home dir");
        let path = "~/some/path";
        let expanded = expand_tilde(path);
        assert!(expanded.starts_with(home.to_str().unwrap()));
        assert!(expanded.ends_with("some/path"));
    }

    #[test]
    fn test_expand_tilde_without_tilde() {
        let path = "/absolute/path";
        let expanded = expand_tilde(path);
        assert_eq!(expanded, path);
    }

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert!(config.db.is_none());
        assert!(config.vault.is_none());
    }

    #[test]
    fn test_config_parse() {
        let json = r#"{"db": "/path/to/db", "vault": "/path/to/vault"}"#;
        let config: Config = json5::from_str(json).unwrap();
        assert_eq!(config.db, Some("/path/to/db".to_string()));
        assert_eq!(config.vault, Some("/path/to/vault".to_string()));
    }

    #[test]
    fn test_config_parse_partial() {
        let json = r#"{"db": "/path/to/db"}"#;
        let config: Config = json5::from_str(json).unwrap();
        assert_eq!(config.db, Some("/path/to/db".to_string()));
        assert!(config.vault.is_none());
    }

    #[test]
    fn test_config_parse_empty() {
        let json = r#"{}"#;
        let config: Config = json5::from_str(json).unwrap();
        assert!(config.db.is_none());
        assert!(config.vault.is_none());
    }

    #[test]
    fn test_config_parse_trailing_comma() {
        let json = r#"{"db": "/path/to/db", "vault": "/path/to/vault",}"#;
        let config: Config = json5::from_str(json).unwrap();
        assert_eq!(config.db, Some("/path/to/db".to_string()));
        assert_eq!(config.vault, Some("/path/to/vault".to_string()));
    }
}
