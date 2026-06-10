use std::{fs, path::PathBuf};

use anyhow::{bail, Context, Result};
use hop_core::{
    encrypt_envelope, new_id, validate_credential_material, AssetAccessMode, AuthType, HopDb,
    MasterKey, NewAsset, NewAuthorizedKey, NewCredential,
};
use russh::keys::{parse_public_key_base64, ssh_key::HashAlg, PublicKeyBase64};

pub fn parse_public_key_line(line: &str) -> Result<(String, String)> {
    let mut parts = line.split_whitespace();
    let Some(key_type) = parts.next() else {
        bail!("empty public key");
    };
    let Some(key_blob) = parts.next() else {
        bail!("public key must be OpenSSH '<type> <base64>' format");
    };
    let key = parse_public_key_base64(key_blob).context("parse OpenSSH public key")?;
    let canonical = format!("{key_type} {}", key.public_key_base64());
    let fingerprint = format!("{}", key.fingerprint(HashAlg::Sha256));
    Ok((canonical, fingerprint))
}

pub async fn add_key(
    db: &HopDb,
    name: String,
    public_key: Option<String>,
    public_key_file: Option<PathBuf>,
) -> Result<()> {
    let key_text = match (public_key, public_key_file) {
        (Some(key), None) => key,
        (None, Some(path)) => {
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?
        }
        (Some(_), Some(_)) => bail!("choose either --public-key or --public-key-file, not both"),
        (None, None) => bail!("--public-key or --public-key-file is required"),
    };
    let (canonical, fingerprint) = parse_public_key_line(&key_text)?;
    let inserted = db
        .add_authorized_key(NewAuthorizedKey::new(name, canonical, fingerprint))
        .await?;
    println!("added key {} {}", inserted.id, inserted.fingerprint);
    Ok(())
}

pub async fn list_keys(db: &HopDb) -> Result<()> {
    for key in db.list_authorized_keys().await? {
        println!(
            "{}\t{}\t{}\t{}",
            key.id,
            if key.is_active { "active" } else { "inactive" },
            key.fingerprint,
            key.name
        );
    }
    Ok(())
}

pub async fn set_key_active(db: &HopDb, id: &str, active: bool) -> Result<()> {
    db.set_authorized_key_active(id, active).await?;
    println!(
        "key {id} {}",
        if active { "activated" } else { "deactivated" }
    );
    Ok(())
}

pub async fn show_key_access(db: &HopDb, id: &str) -> Result<()> {
    print!("{}", format_key_access(db, id).await?);
    Ok(())
}

async fn format_key_access(db: &HopDb, id: &str) -> Result<String> {
    let key = db
        .get_authorized_key_by_id(id)
        .await?
        .with_context(|| format!("unknown authorized key id: {id}"))?;
    let assigned_ids = db.list_asset_ids_for_key(id).await?;
    let mut output = format!(
        "key\t{}\t{}\nmode\t{}\nassigned\t{}\n",
        key.id,
        key.name,
        key.asset_access_mode,
        assigned_ids.len()
    );
    for asset_id in assigned_ids {
        let asset = db
            .get_asset_by_id(&asset_id)
            .await?
            .with_context(|| format!("assigned asset no longer exists: {asset_id}"))?;
        output.push_str(&format!(
            "asset\t{}\t{}\t{}:{}\n",
            asset.id, asset.name, asset.hostname, asset.port
        ));
    }
    Ok(output)
}

pub async fn set_key_access(
    db: &HopDb,
    id: &str,
    mode: AssetAccessMode,
    asset_ids: Vec<String>,
) -> Result<()> {
    if mode == AssetAccessMode::All && !asset_ids.is_empty() {
        bail!("--asset-id can only be used with --mode restricted");
    }
    db.set_authorized_key_access(id, mode, &asset_ids).await?;
    println!(
        "key {id} access mode set to {mode} with {} assigned asset(s)",
        asset_ids.len()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn add_credential(
    db: &HopDb,
    master_key: &MasterKey,
    name: String,
    username: String,
    auth_type: AuthType,
    password: Option<String>,
    private_key_file: Option<PathBuf>,
    passphrase: Option<String>,
) -> Result<()> {
    let id = new_id();
    let password_enc = match password {
        Some(value) => Some(encrypt_envelope(
            master_key,
            &format!("{id}:password"),
            value.as_bytes(),
        )?),
        None => None,
    };
    let private_key_enc = match private_key_file {
        Some(path) => {
            let value =
                fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            Some(encrypt_envelope(
                master_key,
                &format!("{id}:private_key"),
                value.as_bytes(),
            )?)
        }
        None => None,
    };
    let passphrase_enc = match passphrase {
        Some(value) => Some(encrypt_envelope(
            master_key,
            &format!("{id}:passphrase"),
            value.as_bytes(),
        )?),
        None => None,
    };

    validate_credential_material(
        &auth_type,
        password_enc.as_deref(),
        private_key_enc.as_deref(),
        passphrase_enc.as_deref(),
    )?;

    let inserted = db
        .add_credential(NewCredential {
            id: Some(id),
            name,
            username,
            auth_type,
            password_enc,
            private_key_enc,
            passphrase_enc,
        })
        .await?;
    println!(
        "added credential {}\t{}\t{}",
        inserted.id, inserted.auth_type, inserted.name
    );
    Ok(())
}

pub async fn list_credentials(db: &HopDb) -> Result<()> {
    for credential in db.list_credentials().await? {
        println!(
            "{}\t{}\t{}\t{}",
            credential.id, credential.auth_type, credential.username, credential.name
        );
    }
    Ok(())
}

pub async fn delete_credential(db: &HopDb, id: &str) -> Result<()> {
    db.delete_credential(id).await?;
    println!("deleted credential {id}");
    Ok(())
}

pub async fn add_asset(db: &HopDb, asset: NewAsset) -> Result<()> {
    let asset = db.add_asset(asset).await?;
    let kind = asset.preset.as_deref().unwrap_or(&asset.protocol);
    println!(
        "added asset {}\t{}\t{}\t{}:{}",
        asset.id, asset.name, kind, asset.hostname, asset.port
    );
    Ok(())
}

pub async fn list_assets(db: &HopDb) -> Result<()> {
    for asset in db.list_assets().await? {
        let kind = asset.preset.as_deref().unwrap_or(&asset.protocol);
        println!(
            "{}\t{}\t{}\t{}:{}\t{}\t{}",
            asset.id,
            asset.name,
            kind,
            asset.hostname,
            asset.port,
            asset.tags.join(","),
            asset.credential_id.unwrap_or_else(|| "-".to_string())
        );
    }
    Ok(())
}

pub async fn delete_asset(db: &HopDb, id: &str) -> Result<()> {
    db.delete_asset(id).await?;
    println!("deleted asset {id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_public_key_line_returns_sha256_fingerprint() {
        let key =
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ test";
        let (canonical, fingerprint) = parse_public_key_line(key).unwrap();
        assert!(canonical.starts_with("ssh-ed25519 "));
        assert!(fingerprint.starts_with("SHA256:"));
    }

    #[tokio::test]
    async fn key_access_format_lists_mode_and_assignments() {
        let db = HopDb::in_memory().await.unwrap();
        let key = db
            .add_authorized_key(NewAuthorizedKey::new(
                "laptop",
                "ssh-ed25519 AAAA-test",
                "SHA256:test",
            ))
            .await
            .unwrap();
        let asset = db
            .add_asset(NewAsset::new("web", "10.0.0.10", 22))
            .await
            .unwrap();
        set_key_access(
            &db,
            &key.id,
            AssetAccessMode::Restricted,
            vec![asset.id.clone()],
        )
        .await
        .unwrap();

        let output = format_key_access(&db, &key.id).await.unwrap();
        assert!(output.contains("mode\trestricted"));
        assert!(output.contains(&format!("asset\t{}\tweb\t10.0.0.10:22", asset.id)));
    }

    #[tokio::test]
    async fn key_access_rejects_assets_in_all_mode_without_changes() {
        let db = HopDb::in_memory().await.unwrap();
        let key = db
            .add_authorized_key(NewAuthorizedKey::new(
                "laptop",
                "ssh-ed25519 AAAA-test",
                "SHA256:test",
            ))
            .await
            .unwrap();

        assert!(set_key_access(
            &db,
            &key.id,
            AssetAccessMode::All,
            vec!["asset-1".to_string()],
        )
        .await
        .is_err());
        assert_eq!(
            db.get_authorized_key_by_id(&key.id)
                .await
                .unwrap()
                .unwrap()
                .asset_access_mode,
            AssetAccessMode::All
        );
    }
}
