use std::{sync::Arc, time::Duration};

use anyhow::{bail, Context, Result};
use hop_core::{
    decrypt_envelope, validate_tcp_port, Asset, AuthType, Credential, HopDb, MasterKey,
};
use russh::{
    client,
    keys::{decode_secret_key, ssh_key::PublicKey, PrivateKeyWithHashAlg},
    Channel,
};

use super::tofu;

#[derive(Clone)]
pub(crate) struct TofuClient {
    db: HopDb,
    hostname: String,
    port: i64,
}

impl client::Handler for TofuClient {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        tofu::verify_or_learn(&self.db, &self.hostname, self.port, server_public_key).await
    }
}

pub struct ManagedTarget {
    pub session: client::Handle<TofuClient>,
    pub channel: Channel<client::Msg>,
}

pub async fn connect_asset_shell(
    db: HopDb,
    master_key: Arc<MasterKey>,
    asset: &Asset,
    width: u32,
    height: u32,
    timeout: Duration,
) -> Result<ManagedTarget> {
    let credential_id = asset
        .credential_id
        .as_deref()
        .context("asset has no managed credential")?;
    let credential = db
        .get_credential(credential_id)
        .await?
        .context("asset credential not found")?;
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(3600)),
        ..Default::default()
    });
    let handler = TofuClient {
        db: db.clone(),
        hostname: asset.hostname.clone(),
        port: asset.port,
    };
    let port = validate_tcp_port(asset.port)?;
    let addr = (asset.hostname.as_str(), port);
    let mut session = tokio::time::timeout(timeout, client::connect(config, addr, handler))
        .await
        .context("target connect timed out")??;

    authenticate(&mut session, &credential, &master_key).await?;
    let channel = session.channel_open_session().await?;
    channel
        .request_pty(false, "xterm-256color", width, height, 0, 0, &[])
        .await?;
    channel.request_shell(false).await?;
    Ok(ManagedTarget { session, channel })
}

async fn authenticate(
    session: &mut client::Handle<TofuClient>,
    credential: &Credential,
    master_key: &MasterKey,
) -> Result<()> {
    let auth_type =
        AuthType::try_from(credential.auth_type.as_str()).map_err(anyhow::Error::msg)?;
    match auth_type {
        AuthType::Password => {
            let password = decrypt_field(
                master_key,
                credential,
                "password",
                credential.password_enc.as_deref(),
            )?;
            let result = session
                .authenticate_password(credential.username.clone(), password)
                .await?;
            if !result.success() {
                bail!("target password authentication failed");
            }
        }
        AuthType::Key | AuthType::KeyWithPassphrase => {
            let private_key = decrypt_field(
                master_key,
                credential,
                "private_key",
                credential.private_key_enc.as_deref(),
            )?;
            let passphrase = match credential.passphrase_enc.as_deref() {
                Some(_) => Some(decrypt_field(
                    master_key,
                    credential,
                    "passphrase",
                    credential.passphrase_enc.as_deref(),
                )?),
                None => None,
            };
            let key = decode_secret_key(&private_key, passphrase.as_deref())
                .context("decode target private key")?;
            let hash_alg = session.best_supported_rsa_hash().await?.flatten();
            let result = session
                .authenticate_publickey(
                    credential.username.clone(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
                )
                .await?;
            if !result.success() {
                bail!("target public-key authentication failed");
            }
        }
    }
    Ok(())
}

fn decrypt_field(
    master_key: &MasterKey,
    credential: &Credential,
    field: &str,
    envelope: Option<&str>,
) -> Result<String> {
    let envelope = envelope.with_context(|| format!("credential missing {field}"))?;
    let clear = decrypt_envelope(master_key, &format!("{}:{field}", credential.id), envelope)?;
    String::from_utf8(clear).context("credential secret is not valid UTF-8")
}
