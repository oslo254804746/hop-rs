use anyhow::Result;
use hop_core::{HopDb, NewKnownHost};
use russh::keys::{ssh_key::HashAlg, PublicKey};

pub async fn verify_or_learn(
    db: &HopDb,
    hostname: &str,
    port: i64,
    key: &PublicKey,
) -> Result<bool> {
    let key_type = key.algorithm().to_string();
    let fingerprint = format!("{}", key.fingerprint(HashAlg::Sha256));
    match db.get_known_host(hostname, port, &key_type).await? {
        Some(existing) => Ok(existing.fingerprint == fingerprint),
        None => {
            db.upsert_known_host(NewKnownHost {
                hostname: hostname.to_string(),
                port,
                key_type,
                fingerprint,
            })
            .await?;
            Ok(true)
        }
    }
}
