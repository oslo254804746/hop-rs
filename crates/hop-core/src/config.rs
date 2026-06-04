use std::{fs, net::SocketAddr, path::{Path, PathBuf}, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{errors::HopCoreError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HopConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub ssh: SshConfig,
    pub security: SecurityConfig,
    pub runtime: RuntimeConfig,
}

impl Default for HopConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            database: DatabaseConfig::default(),
            ssh: SshConfig::default(),
            security: SecurityConfig::default(),
            runtime: RuntimeConfig::default(),
        }
    }
}

impl HopConfig {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        match path {
            Some(path) if path.exists() => {
                let raw = fs::read_to_string(path)?;
                toml::from_str(&raw).map_err(|err| HopCoreError::Config(err.to_string()))
            }
            Some(path) => Err(HopCoreError::Config(format!("config file not found: {}", path.display()))),
            None => Ok(Self::default()),
        }
    }

    pub fn ssh_bind_addr(&self) -> Result<SocketAddr> {
        self.server
            .ssh_bind
            .parse()
            .map_err(|err| HopCoreError::Config(format!("invalid server.ssh_bind: {err}")))
    }

    pub fn admin_bind_addr(&self) -> Result<SocketAddr> {
        self.server
            .admin_bind
            .parse()
            .map_err(|err| HopCoreError::Config(format!("invalid server.admin_bind: {err}")))
    }

    pub fn connect_timeout(&self) -> Duration {
        Duration::from_secs(self.ssh.connect_timeout)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub ssh_bind: String,
    pub admin_bind: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            ssh_bind: "0.0.0.0:2222".to_string(),
            admin_bind: "127.0.0.1:8080".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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
#[serde(default)]
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
#[serde(default)]
pub struct SecurityConfig {
    pub secret_key_file: PathBuf,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            secret_key_file: PathBuf::from("./hop.secret"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub temp_dir: PathBuf,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            temp_dir: PathBuf::from("/tmp/hop"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_admin_bind_is_localhost() {
        let config = HopConfig::default();
        assert_eq!(config.server.admin_bind, "127.0.0.1:8080");
        assert_eq!(config.server.ssh_bind, "0.0.0.0:2222");
    }
}
