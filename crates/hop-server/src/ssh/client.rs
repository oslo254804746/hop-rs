use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use hop_core::{
    decrypt_envelope, validate_tcp_port, Asset, AuthType, Catalog, Credential, MasterKey,
};
use russh::{
    client,
    keys::{decode_secret_key, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
    Channel, ChannelMsg, ChannelStream,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};
use zeroize::Zeroize;

use super::tofu;

#[derive(Clone)]
pub(crate) struct TofuClient {
    db: Catalog,
    hostname: String,
    port: i64,
}

impl client::Handler for TofuClient {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        tofu::verify_or_learn(
            &self.db,
            &self.hostname,
            self.port,
            &server_public_key.public_key(),
        )
        .await
    }
}

pub enum ClientAuthMaterial {
    StoredCredential,
    InlinePassword {
        username: String,
        password: String,
    },
    InlinePrivateKey {
        username: String,
        private_key_pem: String,
        passphrase: Option<String>,
    },
}

impl Drop for ClientAuthMaterial {
    fn drop(&mut self) {
        match self {
            Self::StoredCredential => {}
            Self::InlinePassword { password, .. } => {
                password.zeroize();
            }
            Self::InlinePrivateKey {
                private_key_pem,
                passphrase,
                ..
            } => {
                private_key_pem.zeroize();
                if let Some(secret) = passphrase {
                    secret.zeroize();
                }
            }
        }
    }
}

pub struct AssetStream {
    inner: AssetStreamInner,
    _upstream_handles: Vec<client::Handle<TofuClient>>,
}

enum AssetStreamInner {
    Direct(TcpStream),
    Tunneled(ChannelStream<client::Msg>),
}

impl AsyncRead for AssetStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut self.get_mut().inner {
            AssetStreamInner::Direct(stream) => Pin::new(stream).poll_read(cx, buf),
            AssetStreamInner::Tunneled(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for AssetStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().inner {
            AssetStreamInner::Direct(stream) => Pin::new(stream).poll_write(cx, buf),
            AssetStreamInner::Tunneled(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().inner {
            AssetStreamInner::Direct(stream) => Pin::new(stream).poll_flush(cx),
            AssetStreamInner::Tunneled(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().inner {
            AssetStreamInner::Direct(stream) => Pin::new(stream).poll_shutdown(cx),
            AssetStreamInner::Tunneled(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

pub struct ManagedClientSession {
    pub session: client::Handle<TofuClient>,
    _upstream_handles: Vec<client::Handle<TofuClient>>,
}

impl ManagedClientSession {
    pub fn is_closed(&self) -> bool {
        self.session.is_closed() || self._upstream_handles.iter().any(client::Handle::is_closed)
    }

    pub async fn disconnect(&self, reason: &str) {
        let _ = self
            .session
            .disconnect(russh::Disconnect::ByApplication, reason, "en")
            .await;
        for handle in self._upstream_handles.iter().rev() {
            let _ = handle
                .disconnect(russh::Disconnect::ByApplication, reason, "en")
                .await;
        }
    }
}

pub struct ManagedTarget {
    pub client: ManagedClientSession,
    pub channel: Channel<client::Msg>,
}

pub fn sanitize_pty_term(term: &str) -> String {
    let trimmed = term.trim();
    if !trimmed.is_empty()
        && trimmed.len() <= 64
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_graphic() && !matches!(ch, '/' | '\\'))
    {
        trimmed.to_string()
    } else {
        "xterm-256color".to_string()
    }
}

pub async fn open_asset_stream(
    db: &Catalog,
    master_key: Option<&MasterKey>,
    asset: &Asset,
    timeout: Duration,
) -> Result<AssetStream> {
    let leaf_port = validate_tcp_port(asset.port)?;
    if asset.via_asset_id.is_none() {
        let stream = tokio::time::timeout(
            timeout,
            TcpStream::connect((asset.hostname.as_str(), leaf_port)),
        )
        .await
        .with_context(|| {
            format!(
                "connect {}:{} timed out after {}ms",
                asset.hostname,
                leaf_port,
                timeout.as_millis()
            )
        })?
        .with_context(|| format!("connect {}:{}", asset.hostname, leaf_port))?;
        let _ = stream.set_nodelay(true);
        return Ok(AssetStream {
            inner: AssetStreamInner::Direct(stream),
            _upstream_handles: Vec::new(),
        });
    }

    let master_key =
        master_key.context("master key is required to traverse via_asset jump chain")?;
    let chain = db.resolve_asset_chain(&asset.id).await?;
    if chain.len() < 2 {
        bail!("asset {} expected a via_asset jump chain", asset.name);
    }
    let hops = &chain[..chain.len() - 1];
    let mut upstream_handles: Vec<client::Handle<TofuClient>> = Vec::with_capacity(hops.len());

    for (idx, hop) in hops.iter().enumerate() {
        let hop_port = validate_tcp_port(hop.port)?;
        let hop_started = Instant::now();
        let hop_result: Result<client::Handle<TofuClient>> = if idx == 0 {
            let tcp = tokio::time::timeout(
                timeout,
                TcpStream::connect((hop.hostname.as_str(), hop_port)),
            )
            .await
            .with_context(|| format!("jump host {} connect timed out", hop.name))?
            .with_context(|| {
                format!(
                    "connect jump host {} ({}:{})",
                    hop.name, hop.hostname, hop_port
                )
            })?;
            let _ = tcp.set_nodelay(true);
            connect_authenticated_over_stream(
                db,
                master_key,
                hop,
                tcp,
                &ClientAuthMaterial::StoredCredential,
                timeout,
            )
            .await
            .with_context(|| format!("authenticate jump host {}", hop.name))
        } else {
            let prev_handle = upstream_handles.last().expect("previous jump hop exists");
            let prev_name = &hops[idx - 1].name;
            let jump_channel = tokio::time::timeout(
                timeout,
                prev_handle.channel_open_direct_tcpip(
                    hop.hostname.clone(),
                    u32::from(hop_port),
                    "127.0.0.1",
                    0,
                ),
            )
            .await
            .with_context(|| format!("jump tunnel from {prev_name} to {} timed out", hop.name))?
            .with_context(|| {
                format!(
                    "open jump tunnel from {prev_name} to {} ({}:{})",
                    hop.name, hop.hostname, hop_port
                )
            })?;
            connect_authenticated_over_stream(
                db,
                master_key,
                hop,
                jump_channel.into_stream(),
                &ClientAuthMaterial::StoredCredential,
                timeout,
            )
            .await
            .with_context(|| format!("authenticate jump host {}", hop.name))
        };

        match hop_result {
            Ok(handle) => {
                let hop_latency = hop_started.elapsed().as_millis().min(i64::MAX as u128) as i64;
                let _ = db
                    .record_asset_health_success(&hop.id, Some(hop_latency))
                    .await;
                upstream_handles.push(handle);
            }
            Err(err) => {
                let msg: String = err.to_string().chars().take(512).collect();
                let _ = db
                    .record_asset_health_failure(
                        &hop.id,
                        Some("managed_connect_failed"),
                        Some(&msg),
                    )
                    .await;
                return Err(err);
            }
        }
    }

    let last_hop = upstream_handles.last().expect("jump hop exists");
    let last_hop_name = &hops.last().expect("jump hop exists").name;
    let leaf_channel = tokio::time::timeout(
        timeout,
        last_hop.channel_open_direct_tcpip(
            asset.hostname.clone(),
            u32::from(leaf_port),
            "127.0.0.1",
            0,
        ),
    )
    .await
    .with_context(|| {
        format!(
            "jump forward via {last_hop_name} to {}:{} timed out",
            asset.hostname, leaf_port
        )
    })?
    .with_context(|| {
        format!(
            "jump forward via {last_hop_name} to {}:{} failed",
            asset.hostname, leaf_port
        )
    })?;

    Ok(AssetStream {
        inner: AssetStreamInner::Tunneled(leaf_channel.into_stream()),
        _upstream_handles: upstream_handles,
    })
}

pub async fn connect_asset_shell(
    db: Catalog,
    master_key: Arc<MasterKey>,
    asset: &Asset,
    term: &str,
    width: u32,
    height: u32,
    timeout: Duration,
) -> Result<ManagedTarget> {
    let client = connect_authenticated_asset(db, master_key, asset, timeout).await?;
    let channel = client.session.channel_open_session().await?;
    let term = sanitize_pty_term(term);
    channel
        .request_pty(false, &term, width, height, 0, 0, &[])
        .await?;
    channel.request_shell(false).await?;
    Ok(ManagedTarget { client, channel })
}

pub async fn connect_asset_exec(
    db: Catalog,
    master_key: Arc<MasterKey>,
    asset: &Asset,
    command: Vec<u8>,
    pty: Option<(&str, u32, u32)>,
    timeout: Duration,
) -> Result<ManagedTarget> {
    let client = connect_authenticated_asset(db, master_key, asset, timeout).await?;
    let mut channel = client.session.channel_open_session().await?;
    if let Some((term, width, height)) = pty {
        let term = sanitize_pty_term(term);
        channel
            .request_pty(true, &term, width, height, 0, 0, &[])
            .await?;
        wait_for_request_success(&mut channel, timeout, "pty").await?;
    }
    channel.exec(true, command).await?;
    wait_for_request_success(&mut channel, timeout, "exec").await?;
    Ok(ManagedTarget { client, channel })
}

pub async fn connect_asset_sftp(
    db: Catalog,
    master_key: Arc<MasterKey>,
    asset: &Asset,
    timeout: Duration,
) -> Result<ManagedTarget> {
    let client = connect_authenticated_asset(db, master_key, asset, timeout).await?;
    let mut channel = client.session.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    wait_for_request_success(&mut channel, timeout, "sftp subsystem").await?;
    Ok(ManagedTarget { client, channel })
}

pub(crate) async fn wait_for_request_success(
    channel: &mut Channel<client::Msg>,
    timeout: Duration,
    request: &str,
) -> Result<()> {
    match tokio::time::timeout(timeout, async {
        loop {
            if request_reply_received(channel.wait().await, request)? {
                return Ok(());
            }
        }
    })
    .await
    {
        Ok(result) => result,
        Err(_) => bail!("target {request} request timed out"),
    }
}

fn request_reply_received(message: Option<ChannelMsg>, request: &str) -> Result<bool> {
    match message {
        Some(ChannelMsg::Success) => Ok(true),
        Some(ChannelMsg::Failure) => bail!("target rejected {request} request"),
        Some(ChannelMsg::WindowAdjusted { .. }) => Ok(false),
        Some(other) => bail!("unexpected target response to {request} request: {other:?}"),
        None => bail!("target closed channel while starting {request}"),
    }
}

pub async fn connect_authenticated_asset(
    db: Catalog,
    master_key: Arc<MasterKey>,
    asset: &Asset,
    timeout: Duration,
) -> Result<ManagedClientSession> {
    connect_asset_with_auth(
        db,
        master_key,
        asset,
        ClientAuthMaterial::StoredCredential,
        timeout,
    )
    .await
}

pub async fn connect_asset_with_auth(
    db: Catalog,
    master_key: Arc<MasterKey>,
    asset: &Asset,
    auth: ClientAuthMaterial,
    timeout: Duration,
) -> Result<ManagedClientSession> {
    if matches!(auth, ClientAuthMaterial::StoredCredential) {
        let credential_id = asset
            .credential_id
            .as_deref()
            .context("asset has no managed credential")?;
        let _ = db
            .get_credential(credential_id)
            .await?
            .context("asset credential not found")?;
    }

    let AssetStream {
        inner,
        _upstream_handles,
    } = open_asset_stream(&db, Some(&master_key), asset, timeout).await?;

    let session = match inner {
        AssetStreamInner::Direct(stream) => {
            connect_authenticated_over_stream(&db, &master_key, asset, stream, &auth, timeout)
                .await?
        }
        AssetStreamInner::Tunneled(stream) => {
            connect_authenticated_over_stream(&db, &master_key, asset, stream, &auth, timeout)
                .await?
        }
    };

    Ok(ManagedClientSession {
        session,
        _upstream_handles,
    })
}

async fn connect_authenticated_over_stream<S>(
    db: &Catalog,
    master_key: &MasterKey,
    asset: &Asset,
    stream: S,
    auth: &ClientAuthMaterial,
    timeout: Duration,
) -> Result<client::Handle<TofuClient>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(3600)),
        nodelay: true,
        ..Default::default()
    });
    let handler = TofuClient {
        db: db.clone(),
        hostname: asset.hostname.clone(),
        port: asset.port,
    };
    let mut session =
        tokio::time::timeout(timeout, client::connect_stream(config, stream, handler))
            .await
            .context("target SSH handshake timed out")??;

    match auth {
        ClientAuthMaterial::StoredCredential => {
            let credential_id = asset
                .credential_id
                .as_deref()
                .context("asset has no managed credential")?;
            let credential = db
                .get_credential(credential_id)
                .await?
                .context("asset credential not found")?;
            authenticate(&mut session, &credential, master_key).await?;
        }
        ClientAuthMaterial::InlinePassword { username, password } => {
            let result = session
                .authenticate_password(username.clone(), password.clone())
                .await?;
            if !result.success() {
                bail!("target password authentication failed");
            }
        }
        ClientAuthMaterial::InlinePrivateKey {
            username,
            private_key_pem,
            passphrase,
        } => {
            let key = decode_secret_key(private_key_pem, passphrase.as_deref())
                .context("decode target private key")?;
            let hash_alg = session.best_supported_rsa_hash().await?.flatten();
            let result = session
                .authenticate_publickey(
                    username.clone(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
                )
                .await?;
            if !result.success() {
                bail!("target public-key authentication failed");
            }
        }
    }
    Ok(session)
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
            let mut password = decrypt_field(
                master_key,
                credential,
                "password",
                credential.password_enc.as_deref(),
            )?;
            let result = session
                .authenticate_password(credential.username.clone(), password.clone())
                .await;
            password.zeroize();
            if !result?.success() {
                bail!("target password authentication failed");
            }
        }
        AuthType::Key | AuthType::KeyWithPassphrase => {
            let mut private_key = decrypt_field(
                master_key,
                credential,
                "private_key",
                credential.private_key_enc.as_deref(),
            )?;
            let mut passphrase = match credential.passphrase_enc.as_deref() {
                Some(_) => Some(decrypt_field(
                    master_key,
                    credential,
                    "passphrase",
                    credential.passphrase_enc.as_deref(),
                )?),
                None => None,
            };
            let key_res = decode_secret_key(&private_key, passphrase.as_deref())
                .context("decode target private key");
            private_key.zeroize();
            if let Some(secret) = &mut passphrase {
                secret.zeroize();
            }
            let key = key_res?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_reply_ignores_informational_window_adjustments() {
        assert!(!request_reply_received(
            Some(ChannelMsg::WindowAdjusted {
                new_size: 2_097_152,
            }),
            "exec",
        )
        .unwrap());
        assert!(request_reply_received(Some(ChannelMsg::Success), "exec").unwrap());
    }

    #[test]
    fn request_reply_preserves_rejection_and_close_errors() {
        assert_eq!(
            request_reply_received(Some(ChannelMsg::Failure), "exec")
                .unwrap_err()
                .to_string(),
            "target rejected exec request"
        );
        assert_eq!(
            request_reply_received(None, "exec")
                .unwrap_err()
                .to_string(),
            "target closed channel while starting exec"
        );
    }
}
