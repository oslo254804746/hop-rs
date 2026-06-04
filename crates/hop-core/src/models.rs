use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuthorizedKey {
    pub id: String,
    pub name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub is_active: bool,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewAuthorizedKey {
    pub name: String,
    pub public_key: String,
    pub fingerprint: String,
}

impl NewAuthorizedKey {
    pub fn new(name: impl Into<String>, public_key: impl Into<String>, fingerprint: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            public_key: public_key.into(),
            fingerprint: fingerprint.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub name: String,
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
    pub hostname: String,
    pub port: i64,
    pub description: Option<String>,
    pub tags: Option<String>,
    pub credential_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl TryFrom<AssetRow> for Asset {
    type Error = serde_json::Error;

    fn try_from(row: AssetRow) -> Result<Self, Self::Error> {
        let tags = match row.tags {
            Some(raw) if !raw.trim().is_empty() => serde_json::from_str(&raw)?,
            _ => Vec::new(),
        };

        Ok(Self {
            id: row.id,
            name: row.name,
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
