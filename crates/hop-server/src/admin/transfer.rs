use anyhow::{bail, Context, Result};
use hop_core::{Asset, AuthType, Credential, HopDb, NewAsset, NewCredential};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    Assets,
    Credentials,
}

impl TransferKind {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "assets" | "asset" => Ok(Self::Assets),
            "credentials" | "credential" => Ok(Self::Credentials),
            other => bail!("unsupported import/export kind: {other}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferFormat {
    Csv,
    Json,
}

impl TransferFormat {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "csv" => Ok(Self::Csv),
            "json" => Ok(Self::Json),
            other => bail!("unsupported import/export format: {other}"),
        }
    }

    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "csv" => Some(Self::Csv),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Csv => "text/csv; charset=utf-8",
            Self::Json => "application/json; charset=utf-8",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    Skip,
    Overwrite,
    Error,
}

impl ConflictPolicy {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "skip" => Ok(Self::Skip),
            "overwrite" => Ok(Self::Overwrite),
            "error" => Ok(Self::Error),
            other => bail!("unsupported conflict policy: {other}"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub imported: usize,
    pub skipped: usize,
    pub overwritten: usize,
    pub errors: Vec<String>,
}

impl ImportSummary {
    pub fn record_imported(&mut self) {
        self.imported += 1;
    }

    pub fn record_skipped(&mut self) {
        self.skipped += 1;
    }

    pub fn record_overwritten(&mut self) {
        self.overwritten += 1;
    }

    pub fn record_error(&mut self, error: impl Into<String>) {
        self.errors.push(error.into());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetTransferRow {
    pub name: String,
    pub hostname: String,
    pub port: i64,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub credential_id: Option<String>,
}

impl From<Asset> for AssetTransferRow {
    fn from(asset: Asset) -> Self {
        Self {
            name: asset.name,
            hostname: asset.hostname,
            port: asset.port,
            description: asset.description,
            tags: asset.tags,
            credential_id: asset.credential_id,
        }
    }
}

impl From<AssetTransferRow> for NewAsset {
    fn from(row: AssetTransferRow) -> Self {
        Self {
            name: row.name,
            hostname: row.hostname,
            port: row.port,
            description: row.description,
            tags: row.tags,
            credential_id: row.credential_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CredentialTransferRow {
    pub name: String,
    pub username: String,
    pub auth_type: String,
}

impl From<Credential> for CredentialTransferRow {
    fn from(credential: Credential) -> Self {
        Self {
            name: credential.name,
            username: credential.username,
            auth_type: credential.auth_type,
        }
    }
}

pub fn export_assets(assets: &[Asset], format: TransferFormat) -> Result<String> {
    let rows = assets
        .iter()
        .cloned()
        .map(AssetTransferRow::from)
        .collect::<Vec<_>>();
    match format {
        TransferFormat::Json => serde_json::to_string(&rows).map_err(Into::into),
        TransferFormat::Csv => Ok(write_asset_csv(&rows)),
    }
}

pub fn export_credentials(credentials: &[Credential], format: TransferFormat) -> Result<String> {
    let rows = credentials
        .iter()
        .cloned()
        .map(CredentialTransferRow::from)
        .collect::<Vec<_>>();
    match format {
        TransferFormat::Json => serde_json::to_string(&rows).map_err(Into::into),
        TransferFormat::Csv => Ok(write_credential_csv(&rows)),
    }
}

pub fn import_asset_rows(input: &str, format: TransferFormat) -> Result<Vec<AssetTransferRow>> {
    match format {
        TransferFormat::Json => serde_json::from_str(input).map_err(Into::into),
        TransferFormat::Csv => read_asset_csv(input),
    }
}

pub fn import_credential_rows(
    input: &str,
    format: TransferFormat,
) -> Result<Vec<CredentialTransferRow>> {
    match format {
        TransferFormat::Json => serde_json::from_str(input).map_err(Into::into),
        TransferFormat::Csv => read_credential_csv(input),
    }
}

pub async fn import_assets(
    db: &HopDb,
    input: &str,
    format: TransferFormat,
    policy: ConflictPolicy,
) -> Result<ImportSummary> {
    let rows = import_asset_rows(input, format)?;
    let mut summary = ImportSummary::default();
    for row in rows {
        match db.get_asset_by_name(&row.name).await? {
            Some(existing) => match policy {
                ConflictPolicy::Skip => summary.record_skipped(),
                ConflictPolicy::Error => {
                    summary.record_error(format!("asset already exists: {}", row.name))
                }
                ConflictPolicy::Overwrite => {
                    db.update_asset(&existing.id, row.into()).await?;
                    summary.record_overwritten();
                }
            },
            None => {
                db.add_asset(row.into()).await?;
                summary.record_imported();
            }
        }
    }
    Ok(summary)
}

pub async fn import_credentials(
    db: &HopDb,
    input: &str,
    format: TransferFormat,
    policy: ConflictPolicy,
) -> Result<ImportSummary> {
    let rows = import_credential_rows(input, format)?;
    let existing = db.list_credentials().await?;
    let mut summary = ImportSummary::default();
    for row in rows {
        let found = existing
            .iter()
            .find(|credential| credential.name == row.name && credential.username == row.username);
        match found {
            Some(credential) => match policy {
                ConflictPolicy::Skip => summary.record_skipped(),
                ConflictPolicy::Error => {
                    summary.record_error(format!("credential already exists: {}", row.name));
                }
                ConflictPolicy::Overwrite => {
                    let auth_type =
                        AuthType::try_from(row.auth_type.as_str()).map_err(anyhow::Error::msg)?;
                    db.update_credential(
                        &credential.id,
                        NewCredential {
                            id: Some(credential.id.clone()),
                            name: row.name,
                            username: row.username,
                            auth_type,
                            password_enc: credential.password_enc.clone(),
                            private_key_enc: credential.private_key_enc.clone(),
                            passphrase_enc: credential.passphrase_enc.clone(),
                        },
                    )
                    .await?;
                    summary.record_overwritten();
                }
            },
            None => {
                let auth_type =
                    AuthType::try_from(row.auth_type.as_str()).map_err(anyhow::Error::msg)?;
                db.add_credential(NewCredential {
                    id: None,
                    name: row.name,
                    username: row.username,
                    auth_type,
                    password_enc: None,
                    private_key_enc: None,
                    passphrase_enc: None,
                })
                .await?;
                summary.record_imported();
            }
        }
    }
    Ok(summary)
}

fn write_asset_csv(rows: &[AssetTransferRow]) -> String {
    let mut output = String::from("name,hostname,port,description,tags,credential_id\n");
    for row in rows {
        push_csv_row(
            &mut output,
            &[
                row.name.as_str(),
                row.hostname.as_str(),
                &row.port.to_string(),
                row.description.as_deref().unwrap_or(""),
                &row.tags.join("|"),
                row.credential_id.as_deref().unwrap_or(""),
            ],
        );
    }
    output
}

fn write_credential_csv(rows: &[CredentialTransferRow]) -> String {
    let mut output = String::from("name,username,auth_type\n");
    for row in rows {
        push_csv_row(
            &mut output,
            &[
                row.name.as_str(),
                row.username.as_str(),
                row.auth_type.as_str(),
            ],
        );
    }
    output
}

fn read_asset_csv(input: &str) -> Result<Vec<AssetTransferRow>> {
    let mut rows = Vec::new();
    for (idx, record) in read_csv_records(input).into_iter().enumerate().skip(1) {
        if record.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        ensure_len(&record, 6, idx)?;
        rows.push(AssetTransferRow {
            name: record[0].clone(),
            hostname: record[1].clone(),
            port: record[2]
                .parse::<i64>()
                .with_context(|| format!("invalid port on CSV row {}", idx + 1))?,
            description: nonempty(record[3].clone()),
            tags: split_tags(&record[4]),
            credential_id: nonempty(record[5].clone()),
        });
    }
    Ok(rows)
}

fn read_credential_csv(input: &str) -> Result<Vec<CredentialTransferRow>> {
    let mut rows = Vec::new();
    for (idx, record) in read_csv_records(input).into_iter().enumerate().skip(1) {
        if record.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        ensure_len(&record, 3, idx)?;
        rows.push(CredentialTransferRow {
            name: record[0].clone(),
            username: record[1].clone(),
            auth_type: record[2].clone(),
        });
    }
    Ok(rows)
}

fn ensure_len(record: &[String], expected: usize, idx: usize) -> Result<()> {
    if record.len() < expected {
        bail!(
            "CSV row {} has {} fields, expected {expected}",
            idx + 1,
            record.len()
        );
    }
    Ok(())
}

fn split_tags(value: &str) -> Vec<String> {
    value
        .split(['|', ','])
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn nonempty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn push_csv_row(output: &mut String, fields: &[&str]) {
    for (idx, field) in fields.iter().enumerate() {
        if idx > 0 {
            output.push(',');
        }
        output.push_str(&escape_csv_field(field));
    }
    output.push('\n');
}

fn escape_csv_field(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

fn read_csv_records(input: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars = input.chars().peekable();
    let mut quoted = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                row.push(std::mem::take(&mut field));
            }
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut row));
            }
            '\r' if !quoted => {}
            _ => field.push(ch),
        }
    }

    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        records.push(row);
    }
    records
}

#[cfg(test)]
mod tests {
    use hop_core::{AuthType, Credential, NewAsset};

    use super::*;

    #[test]
    fn exports_assets_to_csv_with_escaped_tags() {
        let asset = asset("web", "10.0.0.1", &["prod", "web"]);

        let csv = export_assets(&[asset], TransferFormat::Csv).unwrap();

        assert!(csv.starts_with("name,hostname,port,description,tags,credential_id\n"));
        assert!(csv.contains("web,10.0.0.1,22,,prod|web,"));
    }

    #[test]
    fn imports_assets_from_json_rows() {
        let rows = import_asset_rows(
            r#"[{"name":"web","hostname":"10.0.0.1","port":22,"description":null,"tags":["prod"],"credential_id":null}]"#,
            TransferFormat::Json,
        )
        .unwrap();

        assert_eq!(rows[0].name, "web");
        assert_eq!(rows[0].tags, vec!["prod"]);
    }

    #[test]
    fn credential_export_omits_encrypted_secret_fields() {
        let credential = Credential {
            id: "cred-1".to_string(),
            name: "deploy".to_string(),
            username: "deploy".to_string(),
            auth_type: AuthType::Password.as_str().to_string(),
            password_enc: Some("secret".to_string()),
            private_key_enc: Some("secret".to_string()),
            passphrase_enc: Some("secret".to_string()),
            created_at: None,
        };

        let json = export_credentials(&[credential], TransferFormat::Json).unwrap();

        assert!(json.contains("\"name\":\"deploy\""));
        assert!(!json.contains("password_enc"));
        assert!(!json.contains("secret"));
    }

    fn asset(name: &str, hostname: &str, tags: &[&str]) -> hop_core::Asset {
        hop_core::Asset {
            id: name.to_string(),
            name: name.to_string(),
            hostname: hostname.to_string(),
            port: 22,
            description: None,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            credential_id: None,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn import_summary_counts_skips_and_overwrites() {
        let mut summary = ImportSummary::default();
        summary.record_imported();
        summary.record_skipped();
        summary.record_overwritten();

        assert_eq!(summary.imported, 1);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.overwritten, 1);
    }

    #[test]
    fn asset_rows_convert_to_new_assets() {
        let row = AssetTransferRow {
            name: "web".to_string(),
            hostname: "10.0.0.1".to_string(),
            port: 22,
            description: Some("prod web".to_string()),
            tags: vec!["prod".to_string()],
            credential_id: None,
        };

        let new_asset: NewAsset = row.into();

        assert_eq!(new_asset.name, "web");
        assert_eq!(new_asset.tags, vec!["prod"]);
    }

    #[tokio::test]
    async fn credential_metadata_overwrite_preserves_existing_secret_fields() {
        let db = HopDb::in_memory().await.unwrap();
        let existing = db
            .add_credential(NewCredential {
                id: Some("cred-1".to_string()),
                name: "deploy".to_string(),
                username: "deploy".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some("encrypted-password".to_string()),
                private_key_enc: Some("encrypted-key".to_string()),
                passphrase_enc: Some("encrypted-passphrase".to_string()),
            })
            .await
            .unwrap();

        let summary = import_credentials(
            &db,
            "name,username,auth_type\ndeploy,deploy,password\n",
            TransferFormat::Csv,
            ConflictPolicy::Overwrite,
        )
        .await
        .unwrap();

        let updated = db.get_credential(&existing.id).await.unwrap().unwrap();
        assert_eq!(summary.overwritten, 1);
        assert_eq!(updated.password_enc.as_deref(), Some("encrypted-password"));
        assert_eq!(updated.private_key_enc.as_deref(), Some("encrypted-key"));
        assert_eq!(
            updated.passphrase_enc.as_deref(),
            Some("encrypted-passphrase")
        );
    }
}
