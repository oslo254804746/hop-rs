use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::HopCoreError;

pub const ASSET_PROTOCOL_SSH: &str = "ssh";
pub const ASSET_PROTOCOL_TCP: &str = "tcp";
pub const ASSET_PRESET_RDP: &str = "rdp";
pub const ASSET_PRESET_VNC: &str = "vnc";
pub const ASSET_PRESET_MYSQL: &str = "mysql";
pub const ASSET_PRESET_POSTGRES: &str = "postgres";
pub const ASSET_PRESET_REDIS: &str = "redis";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AssetAccessMode {
    #[default]
    All,
    Restricted,
}

impl AssetAccessMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Restricted => "restricted",
        }
    }
}

impl TryFrom<&str> for AssetAccessMode {
    type Error = HopCoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "all" => Ok(Self::All),
            "restricted" => Ok(Self::Restricted),
            other => Err(HopCoreError::Validation(format!(
                "asset access mode must be all or restricted, got {other}"
            ))),
        }
    }
}

impl std::fmt::Display for AssetAccessMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizedKey {
    pub id: String,
    pub name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub is_active: bool,
    pub asset_access_mode: AssetAccessMode,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct AuthorizedKeyRow {
    pub id: String,
    pub name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub is_active: bool,
    pub asset_access_mode: String,
    pub created_at: Option<String>,
}

impl TryFrom<AuthorizedKeyRow> for AuthorizedKey {
    type Error = HopCoreError;

    fn try_from(row: AuthorizedKeyRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            name: row.name,
            public_key: row.public_key,
            fingerprint: row.fingerprint,
            is_active: row.is_active,
            asset_access_mode: AssetAccessMode::try_from(row.asset_access_mode.as_str())?,
            created_at: row.created_at,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewAuthorizedKey {
    pub name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub asset_access_mode: AssetAccessMode,
}

impl NewAuthorizedKey {
    pub fn new(
        name: impl Into<String>,
        public_key: impl Into<String>,
        fingerprint: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            public_key: public_key.into(),
            fingerprint: fingerprint.into(),
            asset_access_mode: AssetAccessMode::All,
        }
    }

    pub fn with_asset_access_mode(mut self, mode: AssetAccessMode) -> Self {
        self.asset_access_mode = mode;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub preset: Option<String>,
    pub hostname: String,
    pub port: i64,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub credential_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct AssetRow {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub preset: Option<String>,
    pub hostname: String,
    pub port: i64,
    pub description: Option<String>,
    pub tags: Option<String>,
    pub credential_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl TryFrom<AssetRow> for Asset {
    type Error = HopCoreError;

    fn try_from(row: AssetRow) -> Result<Self, Self::Error> {
        let tags = match row.tags {
            Some(raw) if !raw.trim().is_empty() => serde_json::from_str(&raw)?,
            _ => Vec::new(),
        };
        let (protocol, preset) = normalize_asset_protocol(&row.protocol, row.preset.as_deref())?;

        Ok(Self {
            id: row.id,
            name: row.name,
            protocol,
            preset,
            hostname: row.hostname,
            port: row.port,
            description: row.description,
            tags,
            credential_id: row.credential_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewAsset {
    pub name: String,
    pub protocol: String,
    pub preset: Option<String>,
    pub hostname: String,
    pub port: i64,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub credential_id: Option<String>,
}

impl NewAsset {
    pub fn new(name: impl Into<String>, hostname: impl Into<String>, port: i64) -> Self {
        Self {
            name: name.into(),
            protocol: ASSET_PROTOCOL_SSH.to_string(),
            preset: None,
            hostname: hostname.into(),
            port,
            description: None,
            tags: Vec::new(),
            credential_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthType {
    Password,
    Key,
    KeyWithPassphrase,
}

impl AuthType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Key => "key",
            Self::KeyWithPassphrase => "key+passphrase",
        }
    }
}

impl TryFrom<&str> for AuthType {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "password" => Ok(Self::Password),
            "key" => Ok(Self::Key),
            "key+passphrase" => Ok(Self::KeyWithPassphrase),
            other => Err(format!("unsupported auth type: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Credential {
    pub id: String,
    pub name: String,
    pub username: String,
    pub auth_type: String,
    pub password_enc: Option<String>,
    pub private_key_enc: Option<String>,
    pub passphrase_enc: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewCredential {
    pub id: Option<String>,
    pub name: String,
    pub username: String,
    pub auth_type: AuthType,
    pub password_enc: Option<String>,
    pub private_key_enc: Option<String>,
    pub passphrase_enc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Session {
    pub id: String,
    pub key_finger: String,
    pub key_name: Option<String>,
    pub mode: String,
    pub asset_name: Option<String>,
    pub target_host: Option<String>,
    pub target_port: Option<i64>,
    pub client_ip: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewSession {
    pub key_finger: String,
    pub key_name: Option<String>,
    pub mode: String,
    pub asset_name: Option<String>,
    pub target_host: Option<String>,
    pub target_port: Option<i64>,
    pub client_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct KnownHost {
    pub hostname: String,
    pub port: i64,
    pub key_type: String,
    pub fingerprint: String,
    pub first_seen: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewKnownHost {
    pub hostname: String,
    pub port: i64,
    pub key_type: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Setting {
    pub key: String,
    pub value: String,
}

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn validate_tcp_port(port: i64) -> crate::Result<u16> {
    match u16::try_from(port) {
        Ok(port) if port > 0 => Ok(port),
        _ => Err(HopCoreError::Validation(format!(
            "tcp port must be between 1 and 65535, got {port}"
        ))),
    }
}

pub fn validate_asset_protocol(protocol: &str) -> crate::Result<String> {
    match protocol.trim().to_ascii_lowercase().as_str() {
        ASSET_PROTOCOL_SSH => Ok(ASSET_PROTOCOL_SSH.to_string()),
        ASSET_PROTOCOL_TCP => Ok(ASSET_PROTOCOL_TCP.to_string()),
        other => Err(HopCoreError::Validation(format!(
            "asset protocol must be ssh or tcp, got {other}"
        ))),
    }
}

pub fn normalize_asset_protocol(
    protocol: &str,
    preset: Option<&str>,
) -> crate::Result<(String, Option<String>)> {
    let protocol = protocol.trim().to_ascii_lowercase();
    let legacy_preset = validate_asset_preset(Some(protocol.as_str()))
        .ok()
        .flatten();
    if legacy_preset.is_some() {
        return Ok((ASSET_PROTOCOL_TCP.to_string(), legacy_preset));
    }
    let protocol = validate_asset_protocol(&protocol)?;
    let preset = validate_asset_preset(preset)?;
    if protocol == ASSET_PROTOCOL_SSH && preset.is_some() {
        return Err(HopCoreError::Validation(
            "ssh assets cannot use a tcp preset".to_string(),
        ));
    }
    Ok((protocol, preset))
}

pub fn validate_asset_preset(preset: Option<&str>) -> crate::Result<Option<String>> {
    let Some(preset) = preset.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    match preset.to_ascii_lowercase().as_str() {
        ASSET_PRESET_RDP => Ok(Some(ASSET_PRESET_RDP.to_string())),
        ASSET_PRESET_VNC => Ok(Some(ASSET_PRESET_VNC.to_string())),
        ASSET_PRESET_MYSQL => Ok(Some(ASSET_PRESET_MYSQL.to_string())),
        ASSET_PRESET_POSTGRES => Ok(Some(ASSET_PRESET_POSTGRES.to_string())),
        ASSET_PRESET_REDIS => Ok(Some(ASSET_PRESET_REDIS.to_string())),
        other => Err(HopCoreError::Validation(format!(
            "unsupported tcp preset: {other}"
        ))),
    }
}

pub fn protocol_supports_managed_credentials(protocol: &str) -> bool {
    protocol == ASSET_PROTOCOL_SSH
}

pub fn validate_credential_material(
    auth_type: &AuthType,
    password_enc: Option<&str>,
    private_key_enc: Option<&str>,
    passphrase_enc: Option<&str>,
) -> crate::Result<()> {
    let has_password = has_secret(password_enc);
    let has_private_key = has_secret(private_key_enc);
    let has_passphrase = has_secret(passphrase_enc);

    match auth_type {
        AuthType::Password if !has_password => Err(HopCoreError::Validation(
            "password auth requires a password".to_string(),
        )),
        AuthType::Key if !has_private_key => Err(HopCoreError::Validation(
            "key auth requires a private key".to_string(),
        )),
        AuthType::KeyWithPassphrase if !has_private_key || !has_passphrase => {
            Err(HopCoreError::Validation(
                "key+passphrase auth requires a private key and passphrase".to_string(),
            ))
        }
        _ => Ok(()),
    }
}

fn has_secret(value: Option<&str>) -> bool {
    value.map(|value| !value.trim().is_empty()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_access_mode_accepts_only_supported_database_values() {
        assert_eq!(
            AssetAccessMode::try_from("all").unwrap(),
            AssetAccessMode::All
        );
        assert_eq!(
            AssetAccessMode::try_from("restricted").unwrap(),
            AssetAccessMode::Restricted
        );
        assert!(AssetAccessMode::try_from("unknown").is_err());
    }

    #[test]
    fn validate_tcp_port_accepts_only_real_tcp_ports() {
        assert_eq!(validate_tcp_port(1).unwrap(), 1);
        assert_eq!(validate_tcp_port(22).unwrap(), 22);
        assert_eq!(validate_tcp_port(65_535).unwrap(), 65_535);

        assert!(validate_tcp_port(0).is_err());
        assert!(validate_tcp_port(-1).is_err());
        assert!(validate_tcp_port(65_536).is_err());
    }

    #[test]
    fn new_asset_defaults_to_ssh_protocol() {
        let asset = NewAsset::new("web-prod-01", "10.0.1.10", 22);

        assert_eq!(asset.protocol, "ssh");
    }

    #[test]
    fn validate_asset_protocol_accepts_only_supported_values() {
        assert_eq!(validate_asset_protocol("ssh").unwrap(), "ssh");
        assert_eq!(validate_asset_protocol(" tcp ").unwrap(), "tcp");
        assert!(validate_asset_protocol("vnc").is_err());
        assert_eq!(
            normalize_asset_protocol("RDP", None).unwrap(),
            ("tcp".to_string(), Some("rdp".to_string()))
        );
        assert_eq!(
            normalize_asset_protocol("tcp", Some("VNC")).unwrap(),
            ("tcp".to_string(), Some("vnc".to_string()))
        );
        assert!(validate_asset_protocol("").is_err());
    }

    #[test]
    fn validate_credential_material_requires_auth_specific_secrets() {
        assert!(validate_credential_material(&AuthType::Password, Some("enc"), None, None).is_ok());
        assert!(validate_credential_material(&AuthType::Password, None, None, None).is_err());

        assert!(validate_credential_material(&AuthType::Key, None, Some("enc"), None).is_ok());
        assert!(validate_credential_material(&AuthType::Key, None, None, None).is_err());

        assert!(validate_credential_material(
            &AuthType::KeyWithPassphrase,
            None,
            Some("enc"),
            Some("enc")
        )
        .is_ok());
        assert!(validate_credential_material(
            &AuthType::KeyWithPassphrase,
            None,
            Some("enc"),
            None
        )
        .is_err());
        assert!(validate_credential_material(
            &AuthType::KeyWithPassphrase,
            None,
            None,
            Some("enc")
        )
        .is_err());
    }
}
