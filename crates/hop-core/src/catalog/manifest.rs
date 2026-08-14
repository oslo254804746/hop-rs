use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::PathBuf,
};

use russh::keys::{decode_secret_key, parse_public_key_base64, ssh_key::HashAlg, PublicKeyBase64};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MANIFEST_API_VERSION: &str = "hop/v1alpha1";

pub type CatalogResult<T> = std::result::Result<T, CatalogError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogErrorCode {
    UnsupportedApiVersion,
    UnknownField,
    DuplicateResource,
    InvalidResourceName,
    InvalidSecretSource,
    SecretUnavailable,
    InvalidCredentialMaterial,
    UnknownReference,
    ResourceInUse,
    OwnershipConflict,
    ManagedBySource,
    RevisionConflict,
    SourceScanIncomplete,
    LegacyDatabaseUnsupported,
    ApplyFailed,
}

impl std::fmt::Display for CatalogErrorCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = serde_json::to_value(self)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "apply_failed".to_string());
        formatter.write_str(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize)]
#[error("{code}: {message}")]
pub struct CatalogError {
    pub code: CatalogErrorCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub message: String,
}

impl CatalogError {
    pub fn new(
        code: CatalogErrorCode,
        path: impl Into<Option<String>>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub api_version: String,
    #[serde(default)]
    pub credentials: BTreeMap<String, CredentialSpec>,
    #[serde(default)]
    pub assets: BTreeMap<String, AssetSpec>,
    #[serde(default)]
    pub access: BTreeMap<String, AccessSpec>,
}

impl Manifest {
    pub fn from_yaml(raw: &str) -> CatalogResult<Self> {
        parse_with_path(serde_yaml::Deserializer::from_str(raw))
    }

    pub fn from_toml(raw: &str) -> CatalogResult<Self> {
        parse_with_path(toml::Deserializer::new(raw))
    }

    pub fn merge(manifests: impl IntoIterator<Item = Manifest>) -> CatalogResult<Self> {
        let mut merged = Self {
            api_version: MANIFEST_API_VERSION.to_string(),
            credentials: BTreeMap::new(),
            assets: BTreeMap::new(),
            access: BTreeMap::new(),
        };
        let mut saw_manifest = false;
        for manifest in manifests {
            saw_manifest = true;
            if manifest.api_version != MANIFEST_API_VERSION {
                return Err(CatalogError::new(
                    CatalogErrorCode::UnsupportedApiVersion,
                    Some("api_version".to_string()),
                    format!("unsupported api_version; expected {MANIFEST_API_VERSION}"),
                ));
            }
            merge_resources(&mut merged.credentials, manifest.credentials, "credentials")?;
            merge_resources(&mut merged.assets, manifest.assets, "assets")?;
            merge_resources(&mut merged.access, manifest.access, "access")?;
        }
        if !saw_manifest {
            return Err(CatalogError::new(
                CatalogErrorCode::SourceScanIncomplete,
                None,
                "manifest scope is empty",
            ));
        }
        Ok(merged)
    }

    pub fn validate_offline(&self) -> CatalogResult<()> {
        let resolved = self.resolve_material()?;
        validate_references(&resolved, false, &BTreeSet::new(), &BTreeSet::new())
    }

    pub(crate) fn resolve_material(&self) -> CatalogResult<ResolvedManifest> {
        if self.api_version != MANIFEST_API_VERSION {
            return Err(CatalogError::new(
                CatalogErrorCode::UnsupportedApiVersion,
                Some("api_version".to_string()),
                format!("unsupported api_version; expected {MANIFEST_API_VERSION}"),
            ));
        }

        let mut credentials = BTreeMap::new();
        for (name, spec) in &self.credentials {
            let path = format!("credentials.{name}");
            validate_name(name, &path)?;
            credentials.insert(name.clone(), resolve_credential(spec, &path)?);
        }

        let mut assets = BTreeMap::new();
        for (name, spec) in &self.assets {
            let path = format!("assets.{name}");
            validate_name(name, &path)?;
            assets.insert(name.clone(), resolve_asset(spec, &path)?);
        }

        let mut access = BTreeMap::new();
        for (name, spec) in &self.access {
            let path = format!("access.{name}");
            validate_name(name, &path)?;
            access.insert(name.clone(), resolve_access(spec, &path)?);
        }

        Ok(ResolvedManifest {
            credentials,
            assets,
            access,
        })
    }
}

fn merge_resources<T>(
    target: &mut BTreeMap<String, T>,
    source: BTreeMap<String, T>,
    resource_type: &str,
) -> CatalogResult<()> {
    for (name, resource) in source {
        if target.insert(name.clone(), resource).is_some() {
            return Err(CatalogError::new(
                CatalogErrorCode::DuplicateResource,
                Some(format!("{resource_type}.{name}")),
                "resource is declared more than once in the apply scope",
            ));
        }
    }
    Ok(())
}

fn parse_with_path<'de, D>(deserializer: D) -> CatalogResult<Manifest>
where
    D: serde::Deserializer<'de>,
{
    serde_path_to_error::deserialize(deserializer).map_err(|error| {
        let parser_message = error.inner().to_string();
        let code = if parser_message.contains("unknown field") {
            CatalogErrorCode::UnknownField
        } else if parser_message.contains("duplicate") {
            CatalogErrorCode::DuplicateResource
        } else {
            CatalogErrorCode::ApplyFailed
        };
        let path = error.path().to_string();
        CatalogError::new(
            code,
            (!path.is_empty()).then_some(path),
            match code {
                CatalogErrorCode::UnknownField => "manifest contains an unknown field",
                CatalogErrorCode::DuplicateResource => "manifest contains a duplicate resource",
                _ => "invalid manifest syntax or field type",
            },
        )
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceState {
    #[default]
    Present,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialType {
    Password,
    SshKey,
}

impl CredentialType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::SshKey => "ssh_key",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialSpec {
    #[serde(default)]
    pub state: ResourceState,
    #[serde(rename = "type")]
    pub credential_type: Option<CredentialType>,
    pub username: Option<String>,
    pub password: Option<SecretSource>,
    pub private_key: Option<SecretSource>,
    pub passphrase: Option<SecretSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    Ssh,
    Tcp,
}

impl AssetType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ssh => "ssh",
            Self::Tcp => "tcp",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSpec {
    #[serde(default)]
    pub state: ResourceState,
    #[serde(rename = "type")]
    pub asset_type: Option<AssetType>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub credential: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessSpec {
    #[serde(default)]
    pub state: ResourceState,
    pub public_key: Option<SecretSource>,
    pub enabled: Option<bool>,
    pub assets: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretSource {
    pub file: Option<PathBuf>,
    pub env: Option<String>,
}

impl SecretSource {
    fn read(&self, path: &str) -> CatalogResult<String> {
        match (&self.file, &self.env) {
            (Some(file), None) => fs::read_to_string(file).map_err(|_| {
                CatalogError::new(
                    CatalogErrorCode::SecretUnavailable,
                    Some(path.to_string()),
                    format!("unable to read secret file {}", file.display()),
                )
            }),
            (None, Some(variable)) if !variable.trim().is_empty() => {
                env::var(variable).map_err(|_| {
                    CatalogError::new(
                        CatalogErrorCode::SecretUnavailable,
                        Some(path.to_string()),
                        format!("secret environment variable {variable} is unavailable"),
                    )
                })
            }
            _ => Err(CatalogError::new(
                CatalogErrorCode::InvalidSecretSource,
                Some(path.to_string()),
                "secret source must set exactly one of file or env",
            )),
        }
    }
}

pub(crate) struct ResolvedManifest {
    pub credentials: BTreeMap<String, ResolvedCredential>,
    pub assets: BTreeMap<String, ResolvedAsset>,
    pub access: BTreeMap<String, ResolvedAccess>,
}

pub(crate) enum ResolvedCredential {
    Absent,
    Present {
        credential_type: CredentialType,
        username: String,
        password: Option<String>,
        private_key: Option<String>,
        passphrase: Option<String>,
    },
}

pub(crate) enum ResolvedAsset {
    Absent,
    Present {
        asset_type: AssetType,
        host: String,
        port: u16,
        display_name: Option<String>,
        description: Option<String>,
        credential: Option<String>,
    },
}

pub(crate) enum ResolvedAccess {
    Absent,
    Present {
        public_key: String,
        fingerprint: String,
        enabled: bool,
        assets: Option<Vec<String>>,
    },
}

fn resolve_credential(spec: &CredentialSpec, path: &str) -> CatalogResult<ResolvedCredential> {
    if spec.state == ResourceState::Absent {
        require_absent_only(
            path,
            [
                spec.credential_type.is_some(),
                spec.username.is_some(),
                spec.password.is_some(),
                spec.private_key.is_some(),
                spec.passphrase.is_some(),
            ],
        )?;
        return Ok(ResolvedCredential::Absent);
    }
    let credential_type = spec
        .credential_type
        .ok_or_else(|| missing_field(format!("{path}.type"), "credential type is required"))?;
    let username = required_text(spec.username.as_deref(), &format!("{path}.username"))?;

    let (password, private_key, passphrase) = match credential_type {
        CredentialType::Password => {
            if spec.private_key.is_some() || spec.passphrase.is_some() {
                return Err(invalid_material(
                    path,
                    "password credential cannot contain private_key or passphrase",
                ));
            }
            let password = spec
                .password
                .as_ref()
                .ok_or_else(|| {
                    missing_field(format!("{path}.password"), "password source is required")
                })?
                .read(&format!("{path}.password"))?;
            if password.is_empty() {
                return Err(invalid_material(
                    &format!("{path}.password"),
                    "password material must not be empty",
                ));
            }
            (Some(password), None, None)
        }
        CredentialType::SshKey => {
            if spec.password.is_some() {
                return Err(invalid_material(
                    path,
                    "ssh_key credential cannot contain password",
                ));
            }
            let passphrase = spec
                .passphrase
                .as_ref()
                .map(|source| source.read(&format!("{path}.passphrase")))
                .transpose()?;
            let private_key = spec
                .private_key
                .as_ref()
                .ok_or_else(|| {
                    missing_field(
                        format!("{path}.private_key"),
                        "private key source is required",
                    )
                })?
                .read(&format!("{path}.private_key"))?;
            decode_secret_key(&private_key, passphrase.as_deref()).map_err(|_| {
                invalid_material(
                    &format!("{path}.private_key"),
                    "private key material is invalid or its passphrase is incorrect",
                )
            })?;
            (None, Some(private_key), passphrase)
        }
    };
    Ok(ResolvedCredential::Present {
        credential_type,
        username,
        password,
        private_key,
        passphrase,
    })
}

fn resolve_asset(spec: &AssetSpec, path: &str) -> CatalogResult<ResolvedAsset> {
    if spec.state == ResourceState::Absent {
        require_absent_only(
            path,
            [
                spec.asset_type.is_some(),
                spec.host.is_some(),
                spec.port.is_some(),
                spec.display_name.is_some(),
                spec.description.is_some(),
                spec.credential.is_some(),
            ],
        )?;
        return Ok(ResolvedAsset::Absent);
    }
    let asset_type = spec
        .asset_type
        .ok_or_else(|| missing_field(format!("{path}.type"), "asset type is required"))?;
    let host = required_text(spec.host.as_deref(), &format!("{path}.host"))?;
    let port = spec
        .port
        .filter(|port| *port > 0)
        .ok_or_else(|| missing_field(format!("{path}.port"), "port must be between 1 and 65535"))?;
    let credential = optional_name(spec.credential.as_deref(), &format!("{path}.credential"))?;
    if asset_type == AssetType::Tcp && credential.is_some() {
        return Err(CatalogError::new(
            CatalogErrorCode::ApplyFailed,
            Some(format!("{path}.credential")),
            "tcp asset cannot reference an ssh credential",
        ));
    }
    Ok(ResolvedAsset::Present {
        asset_type,
        host,
        port,
        display_name: trim_optional(spec.display_name.as_deref()),
        description: trim_optional(spec.description.as_deref()),
        credential,
    })
}

fn resolve_access(spec: &AccessSpec, path: &str) -> CatalogResult<ResolvedAccess> {
    if spec.state == ResourceState::Absent {
        require_absent_only(
            path,
            [
                spec.public_key.is_some(),
                spec.enabled.is_some(),
                spec.assets.is_some(),
            ],
        )?;
        return Ok(ResolvedAccess::Absent);
    }
    let key_text = spec
        .public_key
        .as_ref()
        .ok_or_else(|| {
            missing_field(
                format!("{path}.public_key"),
                "public key source is required",
            )
        })?
        .read(&format!("{path}.public_key"))?;
    let (public_key, fingerprint) = parse_public_key(&key_text, &format!("{path}.public_key"))?;
    let assets = spec
        .assets
        .as_ref()
        .map(|assets| {
            let mut seen = BTreeSet::new();
            let mut normalized = Vec::with_capacity(assets.len());
            for (index, name) in assets.iter().enumerate() {
                validate_name(name, &format!("{path}.assets.{index}"))?;
                if !seen.insert(name.clone()) {
                    return Err(CatalogError::new(
                        CatalogErrorCode::DuplicateResource,
                        Some(format!("{path}.assets.{index}")),
                        "asset appears more than once in the allowlist",
                    ));
                }
                normalized.push(name.clone());
            }
            Ok(normalized)
        })
        .transpose()?;
    Ok(ResolvedAccess::Present {
        public_key,
        fingerprint,
        enabled: spec.enabled.unwrap_or(true),
        assets,
    })
}

pub(crate) fn validate_references(
    manifest: &ResolvedManifest,
    allow_database_references: bool,
    database_credentials: &BTreeSet<String>,
    database_assets: &BTreeSet<String>,
) -> CatalogResult<()> {
    let present_credentials: BTreeSet<&str> = manifest
        .credentials
        .iter()
        .filter_map(|(name, value)| {
            matches!(value, ResolvedCredential::Present { .. }).then_some(name.as_str())
        })
        .collect();
    let present_assets: BTreeSet<&str> = manifest
        .assets
        .iter()
        .filter_map(|(name, value)| {
            matches!(value, ResolvedAsset::Present { .. }).then_some(name.as_str())
        })
        .collect();
    for (name, asset) in &manifest.assets {
        let ResolvedAsset::Present {
            credential: Some(credential),
            ..
        } = asset
        else {
            continue;
        };
        let known = present_credentials.contains(credential.as_str())
            || (allow_database_references && database_credentials.contains(credential));
        if !known
            || matches!(
                manifest.credentials.get(credential),
                Some(ResolvedCredential::Absent)
            )
        {
            return Err(unknown_reference(
                format!("assets.{name}.credential"),
                "credential",
                credential,
            ));
        }
    }
    for (name, access) in &manifest.access {
        let ResolvedAccess::Present {
            assets: Some(assets),
            ..
        } = access
        else {
            continue;
        };
        for (index, asset) in assets.iter().enumerate() {
            let known = present_assets.contains(asset.as_str())
                || (allow_database_references && database_assets.contains(asset));
            if !known || matches!(manifest.assets.get(asset), Some(ResolvedAsset::Absent)) {
                return Err(unknown_reference(
                    format!("access.{name}.assets.{index}"),
                    "asset",
                    asset,
                ));
            }
        }
    }
    Ok(())
}

fn parse_public_key(value: &str, path: &str) -> CatalogResult<(String, String)> {
    let mut parts = value.split_whitespace();
    let key_type = parts.next().ok_or_else(|| {
        invalid_material(path, "public key must use OpenSSH '<type> <base64>' format")
    })?;
    let key_blob = parts.next().ok_or_else(|| {
        invalid_material(path, "public key must use OpenSSH '<type> <base64>' format")
    })?;
    let key = parse_public_key_base64(key_blob)
        .map_err(|_| invalid_material(path, "public key material is invalid"))?;
    if key.algorithm().as_str() != key_type {
        return Err(invalid_material(
            path,
            "public key type does not match its encoded material",
        ));
    }
    Ok((
        format!("{key_type} {}", key.public_key_base64()),
        key.fingerprint(HashAlg::Sha256).to_string(),
    ))
}

fn validate_name(name: &str, path: &str) -> CatalogResult<()> {
    let mut chars = name.chars();
    let first_valid = chars
        .next()
        .map(|character| character.is_ascii_alphanumeric())
        .unwrap_or(false);
    let rest_valid = chars
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'));
    if first_valid && rest_valid && name.len() <= 128 {
        Ok(())
    } else {
        Err(CatalogError::new(
            CatalogErrorCode::InvalidResourceName,
            Some(path.to_string()),
            "resource name must start with an ASCII letter or digit and contain only letters, digits, '.', '-' or '_'",
        ))
    }
}

fn optional_name(value: Option<&str>, path: &str) -> CatalogResult<Option<String>> {
    value
        .map(|value| {
            validate_name(value, path)?;
            Ok(value.to_string())
        })
        .transpose()
}

fn required_text(value: Option<&str>, path: &str) -> CatalogResult<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| missing_field(path.to_string(), "field must not be empty"))
}

fn trim_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn require_absent_only<const N: usize>(path: &str, fields: [bool; N]) -> CatalogResult<()> {
    if fields.into_iter().any(|is_set| is_set) {
        Err(CatalogError::new(
            CatalogErrorCode::ApplyFailed,
            Some(path.to_string()),
            "state absent cannot be combined with resource fields",
        ))
    } else {
        Ok(())
    }
}

fn missing_field(path: String, message: &str) -> CatalogError {
    CatalogError::new(CatalogErrorCode::ApplyFailed, Some(path), message)
}

fn invalid_material(path: &str, message: &str) -> CatalogError {
    CatalogError::new(
        CatalogErrorCode::InvalidCredentialMaterial,
        Some(path.to_string()),
        message,
    )
}

fn unknown_reference(path: String, kind: &str, name: &str) -> CatalogError {
    CatalogError::new(
        CatalogErrorCode::UnknownReference,
        Some(path),
        format!("unknown {kind} reference: {name}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBLIC_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ test";

    #[test]
    fn yaml_and_toml_parse_the_same_strict_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let password = directory.path().join("password");
        let public_key = directory.path().join("id.pub");
        fs::write(&password, "correct horse battery staple").unwrap();
        fs::write(&public_key, PUBLIC_KEY).unwrap();
        let yaml = format!(
            r#"api_version: hop/v1alpha1
credentials:
  root:
    type: password
    username: root
    password:
      file: {}
assets:
  server:
    type: ssh
    host: 192.0.2.10
    port: 22
    credential: root
access:
  laptop:
    public_key:
      file: {}
    assets: [server]
"#,
            password.display(),
            public_key.display()
        );
        let toml = format!(
            r#"api_version = "hop/v1alpha1"

[credentials.root]
type = "password"
username = "root"
password = {{ file = "{}" }}

[assets.server]
type = "ssh"
host = "192.0.2.10"
port = 22
credential = "root"

[access.laptop]
public_key = {{ file = "{}" }}
assets = ["server"]
"#,
            password.display(),
            public_key.display()
        );

        Manifest::from_yaml(&yaml)
            .unwrap()
            .validate_offline()
            .unwrap();
        Manifest::from_toml(&toml)
            .unwrap()
            .validate_offline()
            .unwrap();
    }

    #[test]
    fn unknown_fields_are_rejected_with_a_stable_code() {
        let error = Manifest::from_yaml(
            "api_version: hop/v1alpha1\nassets:\n  demo:\n    type: ssh\n    host: localhost\n    port: 22\n    surprise: true\n",
        )
        .unwrap_err();

        assert_eq!(error.code, CatalogErrorCode::UnknownField);
        assert!(!error.message.contains("true"));
    }

    #[test]
    fn inline_secrets_and_multiple_secret_sources_are_rejected() {
        let inline = Manifest::from_yaml(
            "api_version: hop/v1alpha1\ncredentials:\n  root:\n    type: password\n    username: root\n    password: plaintext\n",
        )
        .unwrap_err();
        assert_eq!(inline.code, CatalogErrorCode::ApplyFailed);
        assert!(!inline.to_string().contains("plaintext"));

        let both = Manifest::from_yaml(
            "api_version: hop/v1alpha1\ncredentials:\n  root:\n    type: password\n    username: root\n    password:\n      file: /tmp/password\n      env: PASSWORD\n",
        )
        .unwrap();
        let error = both.validate_offline().unwrap_err();
        assert_eq!(error.code, CatalogErrorCode::InvalidSecretSource);
    }

    #[test]
    fn offline_validation_rejects_unknown_references_and_absent_fields() {
        let manifest = Manifest::from_yaml(
            "api_version: hop/v1alpha1\nassets:\n  demo:\n    type: ssh\n    host: localhost\n    port: 22\n    credential: missing\n",
        )
        .unwrap();
        let error = manifest.validate_offline().unwrap_err();
        assert_eq!(error.code, CatalogErrorCode::UnknownReference);
        assert_eq!(error.path.as_deref(), Some("assets.demo.credential"));

        let absent = Manifest::from_yaml(
            "api_version: hop/v1alpha1\nassets:\n  demo:\n    state: absent\n    host: localhost\n",
        )
        .unwrap();
        assert_eq!(
            absent.validate_offline().unwrap_err().code,
            CatalogErrorCode::ApplyFailed
        );
    }

    #[test]
    fn access_assets_distinguish_omitted_empty_and_nonempty() {
        let directory = tempfile::tempdir().unwrap();
        let public_key = directory.path().join("id.pub");
        fs::write(&public_key, PUBLIC_KEY).unwrap();
        for (suffix, expected) in [
            ("", None),
            ("    assets: []\n", Some(Vec::new())),
            ("    assets: [demo]\n", Some(vec!["demo".to_string()])),
        ] {
            let yaml = format!(
                "api_version: hop/v1alpha1\nassets:\n  demo:\n    type: tcp\n    host: localhost\n    port: 80\naccess:\n  laptop:\n    public_key:\n      file: {}\n{suffix}",
                public_key.display()
            );
            let resolved = Manifest::from_yaml(&yaml)
                .unwrap()
                .resolve_material()
                .unwrap();
            let ResolvedAccess::Present { assets, .. } = &resolved.access["laptop"] else {
                panic!("access key must be present");
            };
            assert_eq!(assets, &expected);
        }
    }

    #[test]
    fn declarative_assets_reject_preset_aliases() {
        let error = Manifest::from_yaml(
            "api_version: hop/v1alpha1\nassets:\n  console:\n    type: tcp\n    host: 192.0.2.10\n    port: 5900\n    preset: telnet\n",
        )
        .unwrap_err();
        assert_eq!(error.code, CatalogErrorCode::UnknownField);
        assert_eq!(error.path.as_deref(), Some("assets.console.preset"));
    }
}
