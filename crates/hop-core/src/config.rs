use std::{
    collections::BTreeMap,
    fmt, fs,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};

use http::Uri;
use ipnet::IpNet;
use serde::Deserialize;
use zeroize::Zeroize;

use crate::{
    catalog::manifest::{
        AccessSpec, AssetSpec, AssetType, CredentialSpec, CredentialType, ResourceState,
        SecretSource,
    },
    crypto::{encrypt_envelope, MasterKey},
    errors::HopCoreError,
    models::{NewNotificationChannel, NotificationChannelKind, ProbeSettings},
    Manifest, Result, MANIFEST_API_VERSION,
};

#[derive(Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }

    pub fn is_placeholder(&self) -> bool {
        self.0 == "change-me"
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HopConfig {
    pub listen: String,
    pub data_dir: PathBuf,
    pub api: ApiConfig,
    pub ssh: SshConfig,
    pub runtime: RuntimeConfig,
    pub probe: Option<StartupProbeConfig>,
    pub notifications: BTreeMap<String, StartupNotificationConfig>,
    pub credentials: BTreeMap<String, StartupCredentialConfig>,
    pub assets: BTreeMap<String, StartupAssetConfig>,
    pub access_keys: BTreeMap<String, StartupAccessKeyConfig>,
}

impl Default for HopConfig {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:2222".to_string(),
            data_dir: PathBuf::from("."),
            api: ApiConfig::default(),
            ssh: SshConfig::default(),
            runtime: RuntimeConfig::default(),
            probe: None,
            notifications: BTreeMap::new(),
            credentials: BTreeMap::new(),
            assets: BTreeMap::new(),
            access_keys: BTreeMap::new(),
        }
    }
}

impl HopConfig {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let (mut config, base_dir) = match path {
            Some(path) if path.exists() => {
                let raw = fs::read_to_string(path)?;
                let config = match path.extension().and_then(|extension| extension.to_str()) {
                    Some("yaml" | "yml") => {
                        parse_config(serde_yaml::Deserializer::from_str(&raw), "YAML")
                    }
                    Some("toml") => parse_config(toml::Deserializer::new(&raw), "TOML"),
                    _ => Err(HopCoreError::Config(
                        "startup config extension must be .yaml, .yml or .toml".to_string(),
                    )),
                }?;
                let base_dir = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf();
                (config, base_dir)
            }
            Some(path) => {
                return Err(HopCoreError::Config(format!(
                    "config file not found: {}",
                    path.display()
                )))
            }
            None => (Self::default(), std::env::current_dir()?),
        };
        config.resolve_paths(&base_dir);
        config.validate()?;
        Ok(config)
    }

    pub fn ssh_bind_addr(&self) -> Result<SocketAddr> {
        self.listen
            .parse()
            .map_err(|error| HopCoreError::Config(format!("invalid listen: {error}")))
    }

    pub fn api_bind_addr(&self) -> Result<SocketAddr> {
        self.api
            .listen
            .parse()
            .map_err(|error| HopCoreError::Config(format!("invalid api.listen: {error}")))
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("hop.db")
    }

    pub fn master_key_path(&self) -> PathBuf {
        self.data_dir.join("hop.secret")
    }

    pub fn ssh_host_key_path(&self) -> PathBuf {
        self.data_dir.join("hop_host_key")
    }

    pub fn connect_timeout(&self) -> Duration {
        Duration::from_secs(self.ssh.connect_timeout)
    }

    pub fn startup_manifest(&self) -> Manifest {
        let credentials = self
            .credentials
            .iter()
            .map(|(name, credential)| {
                let credential_type = if credential.password.is_some() {
                    CredentialType::Password
                } else {
                    CredentialType::SshKey
                };
                (
                    name.clone(),
                    CredentialSpec {
                        state: ResourceState::Present,
                        credential_type: Some(credential_type),
                        username: Some(credential.username.clone()),
                        password: credential
                            .password
                            .as_ref()
                            .map(|value| SecretSource::inline(value.expose().to_string())),
                        private_key: credential
                            .private_key
                            .as_ref()
                            .map(|value| SecretSource::inline(value.expose().to_string())),
                        passphrase: credential
                            .passphrase
                            .as_ref()
                            .map(|value| SecretSource::inline(value.expose().to_string())),
                    },
                )
            })
            .collect();
        let assets = self
            .assets
            .iter()
            .map(|(name, asset)| {
                let asset_type = asset.asset_type.unwrap_or(AssetType::Ssh);
                (
                    name.clone(),
                    AssetSpec {
                        state: ResourceState::Present,
                        asset_type: Some(asset_type),
                        host: Some(asset.host.clone()),
                        port: Some(asset.port.unwrap_or(22)),
                        display_name: asset.display_name.clone(),
                        description: asset.description.clone(),
                        tags: asset.tags.clone(),
                        credential: asset.credential.clone(),
                        via_asset: asset.via_asset.clone(),
                        allow_tcp_forwarding: asset.allow_tcp_forwarding,
                    },
                )
            })
            .collect();
        let access = self
            .access_keys
            .iter()
            .map(|(name, access_key)| {
                let public_key = match (&access_key.public_key, &access_key.public_key_file) {
                    (Some(value), None) => Some(SecretSource::inline(value.clone())),
                    (None, Some(path)) => Some(SecretSource::file(path.clone())),
                    _ => None,
                };
                (
                    name.clone(),
                    AccessSpec {
                        state: ResourceState::Present,
                        public_key,
                        enabled: access_key.enabled,
                        assets: access_key.assets.clone(),
                    },
                )
            })
            .collect();
        Manifest {
            api_version: MANIFEST_API_VERSION.to_string(),
            credentials,
            assets,
            access,
        }
    }

    fn resolve_paths(&mut self, base_dir: &Path) {
        self.data_dir = resolve_path(base_dir, &self.data_dir);
        self.runtime.temp_dir = resolve_path(base_dir, &self.runtime.temp_dir);
        for access_key in self.access_keys.values_mut() {
            if let Some(path) = &mut access_key.public_key_file {
                *path = resolve_path(base_dir, path);
            }
        }
    }

    fn validate(&self) -> Result<()> {
        self.ssh_bind_addr()?;
        if self.api.enabled {
            self.api_bind_addr()?;
            validate_cors_origins(&self.api.cors_allowlist)?;
        }
        if self.data_dir.as_os_str().is_empty() {
            return Err(HopCoreError::Config(
                "data_dir must not be empty".to_string(),
            ));
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
        validate_allowed_client_ip_ranges(&self.ssh.allowed_client_ip_ranges)?;
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
        if let Some(probe) = &self.probe {
            if !(30..=86_400).contains(&probe.interval_seconds) {
                return Err(HopCoreError::Config(
                    "probe.interval_seconds must be between 30 and 86400".to_string(),
                ));
            }
            if !(500..=30_000).contains(&probe.timeout_ms) {
                return Err(HopCoreError::Config(
                    "probe.timeout_ms must be between 500 and 30000".to_string(),
                ));
            }
            if !(1..=20).contains(&probe.failure_threshold) {
                return Err(HopCoreError::Config(
                    "probe.failure_threshold must be between 1 and 20".to_string(),
                ));
            }
        }
        for (name, channel) in &self.notifications {
            channel.validate(name)?;
        }
        for (name, credential) in &self.credentials {
            let path = format!("credentials.{name}");
            match (&credential.password, &credential.private_key) {
                (Some(_), Some(_)) => {
                    return Err(HopCoreError::Config(format!(
                        "{path}: password and private_key are mutually exclusive"
                    )))
                }
                (None, None) => {
                    return Err(HopCoreError::Config(format!(
                        "{path}: exactly one of password or private_key is required"
                    )))
                }
                _ => {}
            }
            if credential.password.is_some() && credential.passphrase.is_some() {
                return Err(HopCoreError::Config(format!(
                    "{path}.passphrase: passphrase is valid only with private_key"
                )));
            }
        }
        for (name, asset) in &self.assets {
            if asset.asset_type == Some(AssetType::Tcp) && asset.port.is_none() {
                return Err(HopCoreError::Config(format!(
                    "assets.{name}.port: tcp assets require an explicit port"
                )));
            }
        }
        for (name, access_key) in &self.access_keys {
            if access_key.public_key.is_some() && access_key.public_key_file.is_some() {
                return Err(HopCoreError::Config(format!(
                    "access_keys.{name}: public_key and public_key_file are mutually exclusive"
                )));
            }
            if access_key.public_key.is_none() && access_key.public_key_file.is_none() {
                return Err(HopCoreError::Config(format!(
                    "access_keys.{name}: exactly one of public_key or public_key_file is required"
                )));
            }
        }
        self.startup_manifest()
            .validate_offline()
            .map_err(|error| startup_catalog_error(&error))?;
        Ok(())
    }
}

fn parse_config<'de, D>(deserializer: D, format: &str) -> Result<HopConfig>
where
    D: serde::Deserializer<'de>,
{
    serde_path_to_error::deserialize(deserializer).map_err(|error| {
        let path = error.path().to_string();
        let location = if path.is_empty() {
            String::new()
        } else {
            format!(" at {path}")
        };
        HopCoreError::Config(format!("invalid {format} startup config{location}"))
    })
}

fn resolve_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn startup_catalog_error(error: &crate::CatalogError) -> HopCoreError {
    let path = error
        .path
        .as_deref()
        .map(|path| path.replacen("access.", "access_keys.", 1));
    match path {
        Some(path) => HopCoreError::Config(format!("{path}: {}", error.message)),
        None => HopCoreError::Config(error.message.clone()),
    }
}

fn validate_cors_origins(origins: &[String]) -> Result<()> {
    if origins.len() > 1 && origins.iter().any(|origin| origin == "*") {
        return Err(HopCoreError::Config(
            "api.cors_allowlist wildcard '*' must be the only entry".to_string(),
        ));
    }
    for (index, origin) in origins.iter().enumerate() {
        if origin == "*" {
            continue;
        }
        let uri = origin
            .parse::<Uri>()
            .map_err(|_| invalid_origin(index, "must be a valid http:// or https:// Origin"))?;
        let authority_only = origin.split_once("://").is_some_and(|(_, authority)| {
            !authority.is_empty()
                && !authority
                    .chars()
                    .any(|character| matches!(character, '/' | '?' | '#'))
        });
        if !matches!(uri.scheme_str(), Some("http" | "https"))
            || uri.authority().is_none()
            || !authority_only
            || uri
                .authority()
                .is_some_and(|value| value.as_str().contains('@'))
        {
            return Err(invalid_origin(
                index,
                "must include http:// or https:// and a host, with no path, query, or credentials",
            ));
        }
    }
    Ok(())
}

fn invalid_origin(index: usize, guidance: &str) -> HopCoreError {
    HopCoreError::Config(format!("api.cors_allowlist[{index}] {guidance}"))
}

fn validate_allowed_client_ip_ranges(ranges: &[String]) -> Result<()> {
    for (index, range) in ranges.iter().enumerate() {
        range.parse::<IpNet>().map_err(|error| {
            HopCoreError::Config(format!(
                "ssh.allowed_client_ip_ranges[{index}]: invalid CIDR {range:?}: {error}"
            ))
        })?;
    }
    Ok(())
}

fn default_allowed_client_ip_ranges() -> Vec<String> {
    ["0.0.0.0/0", "::/0"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ApiConfig {
    pub enabled: bool,
    pub listen: String,
    pub token: Option<SecretString>,
    pub cors_allowlist: Vec<String>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: "127.0.0.1:8083".to_string(),
            token: None,
            cors_allowlist: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SshConfig {
    pub host_key_type: String,
    pub banner: String,
    pub keepalive_interval: u64,
    pub connect_timeout: u64,
    pub proxy_policy: String,
    #[serde(default = "default_true")]
    pub allow_managed_tcp_forwarding: bool,
    #[serde(default = "default_allowed_client_ip_ranges")]
    pub allowed_client_ip_ranges: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl SshConfig {
    /// Returns whether an SSH peer IP is covered by the configured CIDR ranges.
    ///
    /// Configuration loading validates every range. Invalid ranges are treated as
    /// non-matching here as a fail-closed safeguard for programmatically-created
    /// configurations.
    pub fn allows_client_ip(&self, ip: IpAddr) -> bool {
        self.allowed_client_ip_ranges
            .iter()
            .filter_map(|range| range.parse::<IpNet>().ok())
            .any(|network| match (network, ip) {
                (IpNet::V4(network), IpAddr::V4(ip)) => network.contains(&ip),
                (IpNet::V6(network), IpAddr::V6(ip)) => network.contains(&ip),
                (IpNet::V4(network), IpAddr::V6(ip)) => ip
                    .to_ipv4_mapped()
                    .is_some_and(|mapped_ip| network.contains(&mapped_ip)),
                (IpNet::V6(_), IpAddr::V4(_)) => false,
            })
    }
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            host_key_type: "ed25519".to_string(),
            banner: "Welcome to Hop".to_string(),
            keepalive_interval: 30,
            connect_timeout: 10,
            proxy_policy: "assets_only".to_string(),
            allow_managed_tcp_forwarding: true,
            allowed_client_ip_ranges: default_allowed_client_ip_ranges(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupCredentialConfig {
    pub username: String,
    pub password: Option<SecretString>,
    pub private_key: Option<SecretString>,
    pub passphrase: Option<SecretString>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupAssetConfig {
    #[serde(rename = "type")]
    pub asset_type: Option<AssetType>,
    pub host: String,
    pub port: Option<u16>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub credential: Option<String>,
    pub via_asset: Option<String>,
    pub allow_tcp_forwarding: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupAccessKeyConfig {
    pub public_key: Option<String>,
    pub public_key_file: Option<PathBuf>,
    pub enabled: Option<bool>,
    pub assets: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StartupProbeConfig {
    pub enabled: bool,
    pub interval_seconds: i64,
    pub timeout_ms: i64,
    pub failure_threshold: i64,
    pub notify_on_recovery: bool,
}

impl Default for StartupProbeConfig {
    fn default() -> Self {
        let defaults = ProbeSettings::default();
        Self {
            enabled: defaults.enabled,
            interval_seconds: defaults.interval_seconds,
            timeout_ms: defaults.timeout_ms,
            failure_threshold: defaults.failure_threshold,
            notify_on_recovery: defaults.notify_on_recovery,
        }
    }
}

impl From<&StartupProbeConfig> for ProbeSettings {
    fn from(value: &StartupProbeConfig) -> Self {
        Self {
            enabled: value.enabled,
            interval_seconds: value.interval_seconds,
            timeout_ms: value.timeout_ms,
            failure_threshold: value.failure_threshold,
            notify_on_recovery: value.notify_on_recovery,
            updated_at: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupNotificationConfig {
    #[serde(rename = "type")]
    pub kind: NotificationChannelKind,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub bot_token: Option<SecretString>,
    pub chat_id: Option<String>,
    pub api_base_url: Option<String>,
    pub server_url: Option<String>,
    pub device_key: Option<SecretString>,
    pub url: Option<String>,
    pub secret: Option<SecretString>,
}

impl StartupNotificationConfig {
    pub fn validate(&self, name: &str) -> Result<()> {
        let valid_name = !name.is_empty()
            && name.len() <= 128
            && name
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphanumeric())
            && name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'));
        if !valid_name {
            return Err(HopCoreError::Config(format!(
                "notifications.{name}: name must start with an ASCII letter or digit and contain only letters, digits, '.', '-' or '_'"
            )));
        }
        let path = format!("notifications.{name}");
        match self.kind {
            NotificationChannelKind::Telegram => {
                if self.bot_token.as_ref().is_none_or(SecretString::is_empty) {
                    return Err(HopCoreError::Config(format!(
                        "{path}.bot_token: required for telegram notification channel"
                    )));
                }
                if self
                    .chat_id
                    .as_deref()
                    .map(str::trim)
                    .is_none_or(str::is_empty)
                {
                    return Err(HopCoreError::Config(format!(
                        "{path}.chat_id: required for telegram notification channel"
                    )));
                }
                if let Some(base) = &self.api_base_url {
                    validate_http_url(base, &format!("{path}.api_base_url"))?;
                }
                if self.server_url.is_some()
                    || self.device_key.is_some()
                    || self.url.is_some()
                    || self.secret.is_some()
                {
                    return Err(HopCoreError::Config(format!(
                        "{path}: server_url, device_key, url, and secret are not valid for telegram"
                    )));
                }
            }
            NotificationChannelKind::Bark => {
                if self.device_key.as_ref().is_none_or(SecretString::is_empty) {
                    return Err(HopCoreError::Config(format!(
                        "{path}.device_key: required for bark notification channel"
                    )));
                }
                if let Some(server_url) = &self.server_url {
                    validate_http_url(server_url, &format!("{path}.server_url"))?;
                }
                if self.bot_token.is_some()
                    || self.chat_id.is_some()
                    || self.api_base_url.is_some()
                    || self.url.is_some()
                    || self.secret.is_some()
                {
                    return Err(HopCoreError::Config(format!(
                        "{path}: bot_token, chat_id, api_base_url, url, and secret are not valid for bark"
                    )));
                }
            }
            NotificationChannelKind::Webhook => {
                let Some(url) = &self.url else {
                    return Err(HopCoreError::Config(format!(
                        "{path}.url: required for webhook notification channel"
                    )));
                };
                validate_http_url(url, &format!("{path}.url"))?;
                if self.bot_token.is_some()
                    || self.chat_id.is_some()
                    || self.api_base_url.is_some()
                    || self.server_url.is_some()
                    || self.device_key.is_some()
                {
                    return Err(HopCoreError::Config(format!(
                        "{path}: telegram/bark fields are not valid for webhook"
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn to_encrypted_channel(
        &self,
        existing_id: Option<String>,
        name: &str,
        master_key: &MasterKey,
    ) -> Result<NewNotificationChannel> {
        self.validate(name)?;
        let id = existing_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let json_payload = match self.kind {
            NotificationChannelKind::Telegram => serde_json::json!({
                "bot_token": self.bot_token.as_ref().map(SecretString::expose).unwrap_or_default().trim(),
                "chat_id": self.chat_id.as_deref().unwrap_or_default().trim(),
                "api_base_url": self.api_base_url.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            }),
            NotificationChannelKind::Bark => serde_json::json!({
                "server_url": self.server_url.as_deref().map(str::trim).filter(|s| !s.is_empty()),
                "device_key": self.device_key.as_ref().map(SecretString::expose).unwrap_or_default().trim(),
            }),
            NotificationChannelKind::Webhook => serde_json::json!({
                "url": self.url.as_deref().unwrap_or_default().trim(),
                "secret": self.secret.as_ref().map(SecretString::expose).map(str::trim).filter(|s| !s.is_empty()),
            }),
        };
        let raw_bytes = serde_json::to_vec(&json_payload).map_err(|error| {
            HopCoreError::Config(format!("serialize notification config: {error}"))
        })?;
        let context = crate::models::notification_channel_crypto_context(&id);
        let config_enc = encrypt_envelope(master_key, &context, &raw_bytes)?;
        Ok(NewNotificationChannel {
            id: Some(id),
            name: name.to_string(),
            kind: self.kind,
            enabled: self.enabled,
            config_enc,
        })
    }
}

fn validate_http_url(raw: &str, field_path: &str) -> Result<()> {
    let trimmed = raw.trim();
    let uri = trimmed.parse::<Uri>().map_err(|_| {
        HopCoreError::Config(format!(
            "{field_path}: must be a valid http:// or https:// URL"
        ))
    })?;
    if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.authority().is_none() {
        return Err(HopCoreError::Config(format!(
            "{field_path}: must include http:// or https:// and a host"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
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

    const PUBLIC_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ test";

    #[test]
    fn defaults_disable_http_and_use_expected_listeners() {
        let config = HopConfig::default();
        assert!(!config.api.enabled);
        assert_eq!(config.api.listen, "127.0.0.1:8083");
        assert_eq!(config.listen, "0.0.0.0:2222");
        assert_eq!(
            config.ssh.allowed_client_ip_ranges,
            vec!["0.0.0.0/0", "::/0"]
        );
        assert!(config.ssh.allows_client_ip("192.0.2.10".parse().unwrap()));
        assert!(config.ssh.allows_client_ip("2001:db8::10".parse().unwrap()));
    }

    #[test]
    fn client_ip_ranges_match_ipv4_ipv6_and_ipv4_mapped_ipv6() {
        let ssh = SshConfig {
            allowed_client_ip_ranges: vec!["192.0.2.0/24".to_string(), "2001:db8::/32".to_string()],
            ..SshConfig::default()
        };

        assert!(ssh.allows_client_ip("192.0.2.10".parse().unwrap()));
        assert!(ssh.allows_client_ip("::ffff:192.0.2.10".parse().unwrap()));
        assert!(ssh.allows_client_ip("2001:db8::10".parse().unwrap()));
        assert!(!ssh.allows_client_ip("::192.0.2.10".parse().unwrap()));
        assert!(!ssh.allows_client_ip("192.0.3.10".parse().unwrap()));
        assert!(!ssh.allows_client_ip("2001:db9::10".parse().unwrap()));
    }

    #[test]
    fn explicit_empty_client_ip_ranges_reject_every_address() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("empty.yaml");
        fs::write(&path, "ssh:\n  allowed_client_ip_ranges: []\n").unwrap();

        let config = HopConfig::load(Some(&path)).unwrap();

        assert!(config.ssh.allowed_client_ip_ranges.is_empty());
        assert!(!config.ssh.allows_client_ip("127.0.0.1".parse().unwrap()));
        assert!(!config.ssh.allows_client_ip("::1".parse().unwrap()));
    }

    #[test]
    fn invalid_client_ip_range_reports_the_list_index() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invalid.yaml");
        fs::write(
            &path,
            "ssh:\n  allowed_client_ip_ranges:\n    - 192.0.2.0/24\n    - not-a-cidr\n",
        )
        .unwrap();

        let error = HopConfig::load(Some(&path)).unwrap_err().to_string();

        assert!(error.contains("ssh.allowed_client_ip_ranges[1]"));
        assert!(error.contains("invalid CIDR"));
    }

    #[test]
    fn one_yaml_loads_runtime_resources_and_direct_secrets() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("hop.yaml");
        fs::write(
            &config_path,
            format!(
                r#"listen: 127.0.0.1:12222
data_dir: ./data
api:
  enabled: true
  token: change-me
credentials:
  root:
    username: root
    password: target-password
assets:
  nas:
    host: 192.0.2.10
    display_name: Home NAS
    tags: [lan, storage]
    credential: root
access_keys:
  laptop:
    public_key: "{PUBLIC_KEY}"
    assets: [nas]
"#
            ),
        )
        .unwrap();

        let config = HopConfig::load(Some(&config_path)).unwrap();
        assert_eq!(config.listen, "127.0.0.1:12222");
        assert_eq!(config.data_dir, directory.path().join("data"));
        assert!(config.api.token.as_ref().unwrap().is_placeholder());
        assert_eq!(config.startup_manifest().assets.len(), 1);
        assert_eq!(
            config.assets["nas"].tags,
            vec!["lan".to_string(), "storage".to_string()]
        );
        assert!(!format!("{config:?}").contains("target-password"));
        assert!(!format!("{config:?}").contains("change-me"));
        assert!(!format!("{:?}", config.startup_manifest()).contains("target-password"));
    }

    #[test]
    fn public_key_file_is_relative_to_the_main_config() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("hop.yaml");
        fs::write(directory.path().join("laptop.pub"), PUBLIC_KEY).unwrap();
        fs::write(
            &config_path,
            "access_keys:\n  laptop:\n    public_key_file: ./laptop.pub\n",
        )
        .unwrap();

        let config = HopConfig::load(Some(&config_path)).unwrap();
        assert_eq!(
            config.access_keys["laptop"].public_key_file.as_deref(),
            Some(directory.path().join("laptop.pub").as_path())
        );
    }

    #[test]
    fn public_key_choices_and_direct_secret_shapes_are_strict() {
        let directory = tempfile::tempdir().unwrap();
        for (name, body, expected) in [
            (
                "both.yaml",
                format!(
                    "access_keys:\n  laptop:\n    public_key: '{PUBLIC_KEY}'\n    public_key_file: laptop.pub\n"
                ),
                "mutually exclusive",
            ),
            (
                "neither.yaml",
                "access_keys:\n  laptop:\n    assets: []\n".to_string(),
                "exactly one",
            ),
            (
                "legacy-secret.yaml",
                "credentials:\n  root:\n    username: root\n    password:\n      env: PASSWORD\n"
                    .to_string(),
                "credentials.root.password",
            ),
        ] {
            let path = directory.path().join(name);
            fs::write(&path, body).unwrap();
            assert!(
                HopConfig::load(Some(&path))
                    .unwrap_err()
                    .to_string()
                    .contains(expected)
            );
        }
    }

    #[test]
    fn credential_material_conflicts_are_rejected_with_paths() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hop.yaml");
        fs::write(
            &path,
            "credentials:\n  root:\n    username: root\n    password: one\n    private_key: two\n",
        )
        .unwrap();
        let error = HopConfig::load(Some(&path)).unwrap_err();
        assert!(error.to_string().contains("credentials.root"));
        assert!(!error.to_string().contains("one"));
        assert!(!error.to_string().contains("two"));
    }

    #[test]
    fn non_loopback_api_allows_empty_cors_and_validates_explicit_origins() {
        let directory = tempfile::tempdir().unwrap();
        let same_origin = directory.path().join("same-origin.yaml");
        fs::write(
            &same_origin,
            "api:\n  enabled: true\n  listen: 0.0.0.0:8083\n  cors_allowlist: []\n",
        )
        .unwrap();
        HopConfig::load(Some(&same_origin)).unwrap();

        let explicit = directory.path().join("explicit.yaml");
        fs::write(
            &explicit,
            "api:\n  enabled: true\n  cors_allowlist: ['http://192.168.1.10:8080', 'https://hop.example.com']\n",
        )
        .unwrap();
        HopConfig::load(Some(&explicit)).unwrap();

        for origin in ["192.168.1.10", "https://hop.example.com/path"] {
            let invalid = directory.path().join(format!("{}.yaml", origin.len()));
            fs::write(
                &invalid,
                format!("api:\n  enabled: true\n  cors_allowlist: ['{origin}']\n"),
            )
            .unwrap();
            let error = HopConfig::load(Some(&invalid)).unwrap_err();
            assert!(error.to_string().contains("api.cors_allowlist[0]"));
        }
    }

    #[test]
    fn cors_wildcard_is_valid_only_as_the_sole_entry() {
        let directory = tempfile::tempdir().unwrap();
        let wildcard = directory.path().join("wildcard.yaml");
        fs::write(
            &wildcard,
            "api:\n  enabled: true\n  listen: 0.0.0.0:8083\n  cors_allowlist: ['*']\n",
        )
        .unwrap();
        HopConfig::load(Some(&wildcard)).unwrap();

        let mixed = directory.path().join("mixed.yaml");
        fs::write(
            &mixed,
            "api:\n  enabled: true\n  listen: 0.0.0.0:8083\n  cors_allowlist: ['*', 'https://panel.example']\n",
        )
        .unwrap();
        let error = HopConfig::load(Some(&mixed)).unwrap_err();
        assert!(error.to_string().contains("must be the only entry"));
    }

    #[test]
    fn v0_3_via_asset_forwarding_probe_and_notifications_validate_and_redact() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hop.yaml");
        fs::write(
            &path,
            r#"ssh:
  allow_managed_tcp_forwarding: true
probe:
  enabled: true
  interval_seconds: 180
  timeout_ms: 5000
  failure_threshold: 3
  notify_on_recovery: true
notifications:
  tg-ops:
    type: telegram
    bot_token: "123456:SECRET_BOT_TOKEN"
    chat_id: "-100999"
    api_base_url: "https://api.telegram.org"
  bark-iphone:
    type: bark
    server_url: "https://api.day.app"
    device_key: "BARK_SECRET_KEY"
  ops-hook:
    type: webhook
    url: "https://hooks.example.com/hop"
    secret: "HOOK_SECRET"
credentials:
  root:
    username: root
    password: relay-password
assets:
  hk-relay:
    host: 203.0.113.10
    credential: root
  jp-target:
    host: 198.51.100.20
    credential: root
    via_asset: hk-relay
    allow_tcp_forwarding: false
"#,
        )
        .unwrap();

        let config = HopConfig::load(Some(&path)).unwrap();
        assert!(config.ssh.allow_managed_tcp_forwarding);
        let probe = config.probe.as_ref().unwrap();
        assert!(probe.enabled);
        assert_eq!(probe.interval_seconds, 180);
        assert_eq!(probe.failure_threshold, 3);
        assert_eq!(config.notifications.len(), 3);
        assert_eq!(
            config.assets["jp-target"].via_asset.as_deref(),
            Some("hk-relay")
        );
        assert_eq!(config.assets["jp-target"].allow_tcp_forwarding, Some(false));
        let debug_repr = format!("{config:?}");
        assert!(!debug_repr.contains("SECRET_BOT_TOKEN"));
        assert!(!debug_repr.contains("BARK_SECRET_KEY"));
        assert!(!debug_repr.contains("HOOK_SECRET"));

        let master_key = MasterKey::generate();
        let encrypted = config.notifications["tg-ops"]
            .to_encrypted_channel(None, "tg-ops", &master_key)
            .unwrap();
        assert_eq!(encrypted.kind, NotificationChannelKind::Telegram);
        assert!(!encrypted.config_enc.contains("SECRET_BOT_TOKEN"));

        let cycle = directory.path().join("cycle.yaml");
        fs::write(
            &cycle,
            "credentials:\n  root:\n    username: root\n    password: secret\nassets:\n  a:\n    host: 1.1.1.1\n    credential: root\n    via_asset: b\n  b:\n    host: 2.2.2.2\n    credential: root\n    via_asset: a\n",
        )
        .unwrap();
        let cycle_err = HopConfig::load(Some(&cycle)).unwrap_err().to_string();
        assert!(cycle_err.contains("cycle"));
    }
}
