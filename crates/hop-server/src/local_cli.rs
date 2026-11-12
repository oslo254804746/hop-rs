use std::{fs, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{bail, Context, Result};
use hop_core::{
    encrypt_envelope, new_id, validate_credential_material, Asset, AssetAccessMode, AssetHealth,
    AuthType, Catalog, MasterKey, NewAsset, NewAuditEvent, NewAuthorizedKey, NewCredential,
    ASSET_HEALTH_FAILED, ASSET_HEALTH_HEALTHY, ASSET_PROTOCOL_SSH,
};
use russh::keys::{parse_public_key_base64, ssh_key::HashAlg, PublicKeyBase64};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshConfigMode {
    Relay,
    ProxyJump,
}

async fn record_cli_audit(
    db: &Catalog,
    action: &str,
    target_type: &str,
    target_id: Option<String>,
    target_label: Option<String>,
    details_json: Option<serde_json::Value>,
) {
    let _ = db
        .record_audit_event(NewAuditEvent {
            actor_label: "local-cli".to_string(),
            action: action.to_string(),
            target_type: target_type.to_string(),
            target_id,
            target_label,
            result: "succeeded".to_string(),
            source_ip: None,
            details_json: details_json.map(|value| value.to_string()),
        })
        .await;
}

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
    db: &Catalog,
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
    record_cli_audit(
        db,
        "access_key.create",
        "access_key",
        Some(inserted.id.clone()),
        Some(inserted.name.clone()),
        Some(serde_json::json!({ "fingerprint": inserted.fingerprint })),
    )
    .await;
    println!("added key {} {}", inserted.id, inserted.fingerprint);
    Ok(())
}

pub async fn list_keys(db: &Catalog) -> Result<()> {
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

pub async fn set_key_active(db: &Catalog, id: &str, active: bool) -> Result<()> {
    let existing = db.get_authorized_key_by_id(id).await?;
    db.set_authorized_key_active(id, active).await?;
    record_cli_audit(
        db,
        if active {
            "access_key.activate"
        } else {
            "access_key.deactivate"
        },
        "access_key",
        Some(id.to_string()),
        existing.map(|key| key.name),
        None,
    )
    .await;
    println!(
        "key {id} {}",
        if active { "activated" } else { "deactivated" }
    );
    Ok(())
}

pub async fn delete_key(db: &Catalog, id: &str) -> Result<()> {
    let existing = db.get_authorized_key_by_id(id).await?;
    db.delete_authorized_key(id).await?;
    record_cli_audit(
        db,
        "access_key.delete",
        "access_key",
        Some(id.to_string()),
        existing.map(|key| key.name),
        None,
    )
    .await;
    println!("deleted key {id}");
    Ok(())
}

pub async fn show_key_access(db: &Catalog, id: &str) -> Result<()> {
    print!("{}", format_key_access(db, id).await?);
    Ok(())
}

async fn format_key_access(db: &Catalog, id: &str) -> Result<String> {
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
    db: &Catalog,
    id: &str,
    mode: AssetAccessMode,
    asset_ids: Vec<String>,
) -> Result<()> {
    if mode == AssetAccessMode::All && !asset_ids.is_empty() {
        bail!("--asset-id can only be used with --mode restricted");
    }
    let existing = db.get_authorized_key_by_id(id).await?;
    db.set_authorized_key_access(id, mode, &asset_ids).await?;
    record_cli_audit(
        db,
        "access_key.update_access",
        "access_key",
        Some(id.to_string()),
        existing.map(|key| key.name),
        Some(serde_json::json!({
            "mode": mode.to_string(),
            "assigned_count": asset_ids.len(),
        })),
    )
    .await;
    println!(
        "key {id} access mode set to {mode} with {} assigned asset(s)",
        asset_ids.len()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn add_credential(
    db: &Catalog,
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
    record_cli_audit(
        db,
        "credential.create",
        "credential",
        Some(inserted.id.clone()),
        Some(inserted.name.clone()),
        Some(serde_json::json!({
            "username": inserted.username,
            "auth_type": inserted.auth_type,
        })),
    )
    .await;
    println!(
        "added credential {}\t{}\t{}",
        inserted.id, inserted.auth_type, inserted.name
    );
    Ok(())
}

pub async fn resolve_credential_ref(db: &Catalog, reference: &str) -> Result<String> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        bail!("credential reference must not be empty");
    }
    if let Some(credential) = db.get_credential(trimmed).await? {
        return Ok(credential.id);
    }
    if let Some(credential) = db
        .list_credentials()
        .await?
        .into_iter()
        .find(|credential| credential.name == trimmed)
    {
        return Ok(credential.id);
    }
    bail!("unknown credential id or name: {trimmed}");
}

pub async fn list_credentials(db: &Catalog) -> Result<()> {
    for credential in db.list_credentials().await? {
        println!(
            "{}\t{}\t{}\t{}",
            credential.id, credential.auth_type, credential.username, credential.name
        );
    }
    Ok(())
}

pub async fn delete_credential(db: &Catalog, id: &str) -> Result<()> {
    let existing = db.get_credential(id).await?;
    db.delete_credential(id).await?;
    record_cli_audit(
        db,
        "credential.delete",
        "credential",
        Some(id.to_string()),
        existing.map(|credential| credential.name),
        None,
    )
    .await;
    println!("deleted credential {id}");
    Ok(())
}

pub async fn add_asset(db: &Catalog, asset: NewAsset) -> Result<()> {
    let asset = db.add_asset(asset).await?;
    record_cli_audit(
        db,
        "asset.create",
        "asset",
        Some(asset.id.clone()),
        Some(asset.name.clone()),
        Some(serde_json::json!({
            "protocol": asset.protocol,
            "hostname": asset.hostname,
            "port": asset.port,
            "via_asset_id": asset.via_asset_id,
            "allow_tcp_forwarding": asset.allow_tcp_forwarding,
        })),
    )
    .await;
    println!(
        "added asset {}\t{}\t{}\t{}:{}",
        asset.id, asset.name, asset.protocol, asset.hostname, asset.port
    );
    Ok(())
}

pub async fn resolve_asset_ref(db: &Catalog, reference: &str) -> Result<String> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        bail!("asset reference must not be empty");
    }
    if let Some(asset) = db.get_asset_by_id(trimmed).await? {
        return Ok(asset.id);
    }
    if let Some(asset) = db.get_asset_by_name(trimmed).await? {
        return Ok(asset.id);
    }
    bail!("unknown asset id or name: {trimmed}");
}

pub fn format_health_column(health: &AssetHealth) -> String {
    match health.status.as_str() {
        ASSET_HEALTH_HEALTHY => match health.latency_ms {
            Some(ms) => format!("healthy:{ms}ms"),
            None => "healthy".to_string(),
        },
        ASSET_HEALTH_FAILED => match health.error_code.as_deref() {
            Some(code) => format!("failed:{code}"),
            None => "failed".to_string(),
        },
        _ => "unknown".to_string(),
    }
}

pub async fn list_assets(db: &Catalog) -> Result<()> {
    for asset in db.list_assets().await? {
        println!(
            "{}\t{}\t{}\t{}:{}\t{}\t{}\t{}\t{}",
            asset.id,
            asset.name,
            asset.protocol,
            asset.hostname,
            asset.port,
            asset.tags.join(","),
            asset.credential_id.unwrap_or_else(|| "-".to_string()),
            asset.via_asset_id.unwrap_or_else(|| "-".to_string()),
            format_health_column(&asset.health)
        );
    }
    Ok(())
}

pub async fn delete_asset(db: &Catalog, id: &str) -> Result<()> {
    let existing = db.get_asset_by_id(id).await?;
    db.delete_asset(id).await?;
    record_cli_audit(
        db,
        "asset.delete",
        "asset",
        Some(id.to_string()),
        existing.map(|asset| asset.name),
        None,
    )
    .await;
    println!("deleted asset {id}");
    Ok(())
}

pub async fn probe_assets(
    db: &Catalog,
    master_key: Option<std::sync::Arc<MasterKey>>,
    target: Option<&str>,
    timeout: Duration,
) -> Result<()> {
    let results = if let Some(reference) = target {
        let trimmed = reference.trim();
        let asset = match db.get_asset_by_id(trimmed).await? {
            Some(asset) => asset,
            None => db
                .get_asset_by_name(trimmed)
                .await?
                .with_context(|| format!("unknown asset id or name: {trimmed}"))?,
        };
        let probed = crate::probe::probe_asset(db, master_key.as_deref(), &asset, timeout).await?;
        record_cli_audit(
            db,
            "asset.probe",
            "asset",
            Some(probed.id.clone()),
            Some(probed.name.clone()),
            Some(serde_json::json!({
                "status": probed.health.status,
                "latency_ms": probed.health.latency_ms,
                "error_code": probed.health.error_code,
            })),
        )
        .await;
        vec![probed]
    } else {
        let probed = crate::probe::probe_all_assets(db, master_key, timeout).await?;
        let healthy = probed
            .iter()
            .filter(|asset| asset.health.status == ASSET_HEALTH_HEALTHY)
            .count();
        record_cli_audit(
            db,
            "asset.probe_all",
            "asset",
            None,
            None,
            Some(serde_json::json!({
                "total": probed.len(),
                "healthy": healthy,
            })),
        )
        .await;
        probed
    };

    for asset in results {
        let detail = match asset.health.status.as_str() {
            ASSET_HEALTH_HEALTHY => asset
                .health
                .latency_ms
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_else(|| "ok".to_string()),
            ASSET_HEALTH_FAILED => asset
                .health
                .error_message
                .unwrap_or_else(|| "probe failed".to_string()),
            _ => "unknown".to_string(),
        };
        println!(
            "{}\t{}\t{}:{}\t{}\t{}",
            asset.name, asset.protocol, asset.hostname, asset.port, asset.health.status, detail
        );
    }
    Ok(())
}

pub async fn bootstrap_asset_key(
    db: &Catalog,
    master_key: Arc<MasterKey>,
    target: &str,
    options: crate::bootstrap::BootstrapKeyOptions,
    timeout: Duration,
) -> Result<()> {
    let asset_id = resolve_asset_ref(db, target).await?;
    let outcome =
        crate::bootstrap::bootstrap_asset_ssh_key(db, master_key, &asset_id, options, timeout)
            .await?;
    record_cli_audit(
        db,
        "asset.bootstrap_key",
        "asset",
        Some(outcome.asset.id.clone()),
        Some(outcome.asset.name.clone()),
        Some(serde_json::json!({
            "credential_id": outcome.credential.id,
            "fingerprint": outcome.public_key_fingerprint,
            "pruned_previous_credential": outcome.pruned_previous_credential,
            "password_auth_disabled": outcome.password_auth_disabled,
        })),
    )
    .await;
    println!(
        "bootstrapped {} ({}:{}) -> credential {} ({})",
        outcome.asset.name,
        outcome.asset.hostname,
        outcome.asset.port,
        outcome.credential.name,
        outcome.public_key_fingerprint
    );
    if outcome.password_auth_disabled {
        println!("sshd PasswordAuthentication disabled and reloaded");
    }
    for warning in &outcome.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

pub async fn list_known_hosts(db: &Catalog) -> Result<()> {
    for host in db.list_known_hosts().await? {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            host.hostname,
            host.port,
            host.key_type,
            host.fingerprint,
            host.first_seen.unwrap_or_else(|| "-".to_string())
        );
    }
    Ok(())
}

pub async fn reset_known_host(
    db: &Catalog,
    hostname: &str,
    port: i64,
    key_type: &str,
) -> Result<()> {
    let deleted = db.delete_known_host(hostname, port, key_type).await?;
    if !deleted {
        bail!("known_host entry not found for {hostname}:{port} ({key_type})");
    }
    record_cli_audit(
        db,
        "known_host.reset",
        "known_host",
        None,
        Some(format!("{hostname}:{port}")),
        Some(serde_json::json!({ "key_type": key_type })),
    )
    .await;
    println!("reset known_host {hostname}:{port} ({key_type})");
    Ok(())
}

pub async fn list_audit_events(db: &Catalog, limit: i64) -> Result<()> {
    let bounded = limit.clamp(1, 500);
    for event in db.list_audit_events(bounded).await? {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            event.occurred_at.unwrap_or_else(|| "-".to_string()),
            event.actor_label,
            event.action,
            event.target_type,
            event.target_label.unwrap_or_else(|| "-".to_string()),
            event.result
        );
    }
    Ok(())
}

pub fn render_ssh_config(
    assets: &[Asset],
    hop_host: &str,
    hop_port: u16,
    identity_file: Option<&str>,
    mode: SshConfigMode,
) -> Result<String> {
    let hop_host = hop_host.trim();
    if hop_host.is_empty() {
        bail!("--host must not be empty");
    }
    let identity_line = identity_file
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(|path| format!("  IdentityFile {path}\n  IdentitiesOnly yes\n"))
        .unwrap_or_default();

    let name_by_id: std::collections::HashMap<&str, &str> = assets
        .iter()
        .map(|asset| (asset.id.as_str(), asset.name.as_str()))
        .collect();

    let mut out = String::new();
    out.push_str("# Generated by hop-server ssh-config\n");
    if mode == SshConfigMode::Relay {
        out.push_str(
            "# Managed Vault mode: supports SSH, SFTP, -L/-D forwarding, and Cursor / VS Code Remote-SSH\n",
        );
    }
    out.push_str("# Interactive TUI selector: ssh hop\n");
    out.push_str("Host hop\n");
    out.push_str(&format!("  HostName {hop_host}\n"));
    out.push_str(&format!("  Port {hop_port}\n"));
    out.push_str("  User menu\n");
    out.push_str(&identity_line);

    if mode == SshConfigMode::ProxyJump {
        out.push_str("\n# End-to-end ProxyJump gateway for Cursor / VS Code Remote-SSH\n");
        out.push_str("Host hop-gateway\n");
        out.push_str(&format!("  HostName {hop_host}\n"));
        out.push_str(&format!("  Port {hop_port}\n"));
        out.push_str("  User proxy\n");
        out.push_str(&identity_line);
    }

    for asset in assets {
        let via_suffix = asset
            .via_asset_id
            .as_deref()
            .and_then(|via_id| name_by_id.get(via_id).copied())
            .map(|via_name| format!(" via {via_name}"))
            .unwrap_or_default();
        if asset.protocol == ASSET_PROTOCOL_SSH {
            out.push('\n');
            if let Some(label) = asset.display_name.as_deref() {
                out.push_str(&format!(
                    "# {label} ({}:{}{via_suffix})\n",
                    asset.hostname, asset.port
                ));
            } else if !via_suffix.is_empty() {
                out.push_str(&format!(
                    "# {} ({}:{}{via_suffix})\n",
                    asset.name, asset.hostname, asset.port
                ));
            }
            match mode {
                SshConfigMode::Relay => {
                    out.push_str(&format!("Host {}\n", asset.name));
                    out.push_str(&format!("  HostName {hop_host}\n"));
                    out.push_str(&format!("  Port {hop_port}\n"));
                    out.push_str(&format!("  User {}\n", asset.name));
                    out.push_str(&identity_line);
                }
                SshConfigMode::ProxyJump => {
                    out.push_str(&format!("Host {}\n", asset.name));
                    out.push_str(&format!("  HostName {}.hop\n", asset.name));
                    out.push_str(&format!("  Port {}\n", asset.port));
                    out.push_str("  ProxyJump hop-gateway\n");
                }
            }
        } else {
            out.push_str(&format!(
                "\n# TCP tunnel ({}{via_suffix}): ssh -N -L {}:{}.hop:{} proxy@{} -p {}\n",
                asset.name, asset.port, asset.name, asset.port, hop_host, hop_port
            ));
        }
    }

    Ok(out)
}

pub async fn print_ssh_config(
    db: &Catalog,
    hop_host: &str,
    hop_port: u16,
    identity_file: Option<&str>,
    mode: SshConfigMode,
) -> Result<()> {
    let assets = db.list_assets().await?;
    let rendered = render_ssh_config(&assets, hop_host, hop_port, identity_file, mode)?;
    print!("{rendered}");
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
        let db = Catalog::in_memory().await.unwrap();
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
        assert_eq!(db.list_audit_events(10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn key_access_rejects_assets_in_all_mode_without_changes() {
        let db = Catalog::in_memory().await.unwrap();
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

    #[tokio::test]
    async fn resolve_credential_ref_accepts_id_or_name() {
        let db = Catalog::in_memory().await.unwrap();
        let master_key = MasterKey::generate();
        add_credential(
            &db,
            &master_key,
            "prod-root".to_string(),
            "root".to_string(),
            AuthType::Password,
            Some("secret".to_string()),
            None,
            None,
        )
        .await
        .unwrap();
        let cred = db.list_credentials().await.unwrap().remove(0);
        assert_eq!(
            resolve_credential_ref(&db, &cred.id).await.unwrap(),
            cred.id
        );
        assert_eq!(
            resolve_credential_ref(&db, "prod-root").await.unwrap(),
            cred.id
        );
        assert!(resolve_credential_ref(&db, "missing").await.is_err());
    }

    #[tokio::test]
    async fn render_ssh_config_supports_relay_and_proxyjump_modes() {
        let db = Catalog::in_memory().await.unwrap();
        let mut ssh_asset = NewAsset::new("hk-cn2", "203.0.113.10", 22022);
        ssh_asset.display_name = Some("HK CN2".to_string());
        db.add_asset(ssh_asset).await.unwrap();
        let mut tcp_asset = NewAsset::new("prod-redis", "10.0.0.15", 6379);
        tcp_asset.protocol = "tcp".to_string();
        db.add_asset(tcp_asset).await.unwrap();
        let assets = db.list_assets().await.unwrap();

        let relay = render_ssh_config(
            &assets,
            "hop.example.com",
            2222,
            Some("~/.ssh/id_ed25519"),
            SshConfigMode::Relay,
        )
        .unwrap();
        assert!(relay.contains("Host hop\n  HostName hop.example.com\n  Port 2222\n  User menu"));
        assert!(
            relay.contains("Host hk-cn2\n  HostName hop.example.com\n  Port 2222\n  User hk-cn2")
        );
        assert!(relay.contains("6379:prod-redis.hop:6379"));

        let proxy = render_ssh_config(
            &assets,
            "hop.example.com",
            2222,
            Some("~/.ssh/id_ed25519"),
            SshConfigMode::ProxyJump,
        )
        .unwrap();
        assert!(proxy.contains("Host hop-gateway"));
        assert!(proxy
            .contains("Host hk-cn2\n  HostName hk-cn2.hop\n  Port 22022\n  ProxyJump hop-gateway"));
    }
}
