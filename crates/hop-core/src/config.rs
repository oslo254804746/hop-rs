use std::{
    collections::BTreeSet,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::{errors::HopCoreError, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HopConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub api: ApiConfig,
    pub ssh: SshConfig,
    pub security: SecurityConfig,
    pub inventory: InventoryConfig,
    pub runtime: RuntimeConfig,
}

impl HopConfig {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let config = match path {
            Some(path) if path.exists() => {
                let raw = fs::read_to_string(path)?;
                match path.extension().and_then(|extension| extension.to_str()) {
                    Some("yaml" | "yml") => serde_yaml::from_str(&raw).map_err(|_| {
                        HopCoreError::Config("invalid YAML startup config".to_string())
                    }),
                    Some("toml") => toml::from_str(&raw).map_err(|_| {
                        HopCoreError::Config("invalid TOML startup config".to_string())
                    }),
                    _ => Err(HopCoreError::Config(
                        "startup config extension must be .yaml, .yml or .toml".to_string(),
                    )),
                }
            }
            Some(path) => Err(HopCoreError::Config(format!(
                "config file not found: {}",
                path.display()
            ))),
            None => Ok(Self::default()),
        }?;
        config.validate()?;
        Ok(config)
    }

    pub fn ssh_bind_addr(&self) -> Result<SocketAddr> {
        self.server
            .ssh_listen
            .parse()
            .map_err(|err| HopCoreError::Config(format!("invalid server.ssh_listen: {err}")))
    }

    pub fn api_bind_addr(&self) -> Result<SocketAddr> {
        self.api
            .listen
            .parse()
            .map_err(|err| HopCoreError::Config(format!("invalid api.listen: {err}")))
    }

    pub fn connect_timeout(&self) -> Duration {
        Duration::from_secs(self.ssh.connect_timeout)
    }

    fn validate(&self) -> Result<()> {
        self.ssh_bind_addr()?;
        if self.api.enabled {
            let api_bind = self.api_bind_addr()?;
            if !api_bind.ip().is_loopback() && self.api.cors_allowlist.is_empty() {
                return Err(HopCoreError::Config(
                    "api.cors_allowlist must be explicit when api.listen is not loopback"
                        .to_string(),
                ));
            }
        }
        if self.ssh.host_key_type != "ed25519" {
            return Err(HopCoreError::Config(
                "ssh.host_key_type must be ed25519".to_string(),
            ));
        }
        if self.ssh.proxy_policy != "assets_only" {
            return Err(HopCoreError::Config(
                "ssh.proxy_policy must be assets_only".to_string(),
            ));
        }
        if self.api.enabled && self.api.token_file.as_os_str().is_empty() {
            return Err(HopCoreError::Config(
                "api.token_file is required when api.enabled is true".to_string(),
            ));
        }
        if self.runtime.temp_dir.as_os_str().is_empty() {
            return Err(HopCoreError::Config(
                "runtime.temp_dir must not be empty".to_string(),
            ));
        }
        if self.runtime.log_level.trim().is_empty() {
            return Err(HopCoreError::Config(
                "runtime.log_level must not be empty".to_string(),
            ));
        }
        let mut source_ids = BTreeSet::new();
        for (index, source) in self.inventory.sources.iter().enumerate() {
            let valid_id = !source.id.is_empty()
                && source.id.len() <= 128
                && source.id.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
                });
            if !valid_id {
                return Err(HopCoreError::Config(format!(
                    "inventory.sources[{index}].id must contain only ASCII letters, digits, '.', '-' or '_'"
                )));
            }
            if !source_ids.insert(&source.id) {
                return Err(HopCoreError::Config(format!(
                    "duplicate inventory source id: {}",
                    source.id
                )));
            }
            if source.path.trim().is_empty() {
                return Err(HopCoreError::Config(format!(
                    "inventory.sources[{index}].path must not be empty"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub ssh_listen: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            ssh_listen: "0.0.0.0:2222".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ApiConfig {
    pub enabled: bool,
    pub listen: String,
    pub token_file: PathBuf,
    pub cors_allowlist: Vec<String>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: "127.0.0.1:8083".to_string(),
            token_file: PathBuf::from("./hop-api.token"),
            cors_allowlist: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    pub path: PathBuf,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("./hop.db"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SshConfig {
    pub host_key_file: PathBuf,
    pub host_key_type: String,
    pub banner: String,
    pub keepalive_interval: u64,
    pub connect_timeout: u64,
    pub proxy_policy: String,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            host_key_file: PathBuf::from("./hop_host_key"),
            host_key_type: "ed25519".to_string(),
            banner: "Welcome to Hop".to_string(),
            keepalive_interval: 30,
            connect_timeout: 10,
            proxy_policy: "assets_only".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecurityConfig {
    pub master_key_file: PathBuf,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            master_key_file: PathBuf::from("./hop.secret"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InventoryConfig {
    pub sources: Vec<InventorySourceConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InventorySourceConfig {
    pub id: String,
    pub path: String,
    pub watch: bool,
    pub prune: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    pub temp_dir: PathBuf,
    pub log_level: String,
    pub session_retention_days: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            temp_dir: PathBuf::from("/tmp/hop"),
            log_level: "info".to_string(),
            session_retention_days: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_disable_http_and_use_loopback_when_enabled() {
        let config = HopConfig::default();
        assert!(!config.api.enabled);
        assert_eq!(config.api.listen, "127.0.0.1:8083");
        assert_eq!(config.server.ssh_listen, "0.0.0.0:2222");
    }

    #[test]
    fn yaml_and_toml_load_the_same_startup_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let yaml = directory.path().join("hop.yaml");
        let toml = directory.path().join("hop.toml");
        fs::write(
            &yaml,
            "server:\n  ssh_listen: 127.0.0.1:12222\napi:\n  enabled: true\n",
        )
        .unwrap();
        fs::write(
            &toml,
            "[server]\nssh_listen = '127.0.0.1:12222'\n[api]\nenabled = true\n",
        )
        .unwrap();

        let yaml_config = HopConfig::load(Some(&yaml)).unwrap();
        let toml_config = HopConfig::load(Some(&toml)).unwrap();
        assert_eq!(yaml_config.server.ssh_listen, toml_config.server.ssh_listen);
        assert!(yaml_config.api.enabled);
        assert!(toml_config.api.enabled);
    }

    #[test]
    fn startup_config_rejects_duplicate_inventory_sources() {
        let directory = tempfile::tempdir().unwrap();
        let yaml = directory.path().join("hop.yaml");
        fs::write(
            &yaml,
            "inventory:\n  sources:\n    - { id: home, path: a.yaml }\n    - { id: home, path: b.yaml }\n",
        )
        .unwrap();
        let error = HopConfig::load(Some(&yaml)).unwrap_err();
        assert!(error.to_string().contains("duplicate inventory source id"));
    }

    #[test]
    fn startup_config_rejects_invalid_listener_before_runtime_use() {
        let directory = tempfile::tempdir().unwrap();
        let yaml = directory.path().join("hop.yaml");
        fs::write(&yaml, "server:\n  ssh_listen: not-an-address\n").unwrap();
        let error = HopConfig::load(Some(&yaml)).unwrap_err();
        assert!(error.to_string().contains("invalid server.ssh_listen"));
    }

    #[test]
    fn non_loopback_api_requires_an_explicit_cors_allowlist() {
        let directory = tempfile::tempdir().unwrap();
        let yaml = directory.path().join("hop.yaml");
        fs::write(
            &yaml,
            "api:\n  enabled: true\n  listen: 0.0.0.0:8083\n  cors_allowlist: []\n",
        )
        .unwrap();
        let error = HopConfig::load(Some(&yaml)).unwrap_err();
        assert!(error.to_string().contains("api.cors_allowlist"));
    }
}
