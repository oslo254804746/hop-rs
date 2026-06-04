use std::{collections::HashMap, io, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use hop_core::{Asset, HopConfig, HopDb, MasterKey, NewSession};
use russh::{
    keys::{ssh_key::HashAlg, PublicKey},
    server::{self, Msg, Server as _, Session},
    Channel, ChannelId, Pty,
};
use tokio::{net::TcpListener, sync::Mutex};
use tracing::{error, info, warn};

use crate::tui::{TuiAction, TuiResources};

use super::{
    bridge::{self, BridgeControl, ManagedBridgeOptions, SharedChannels},
    host_key, proxy,
    routes::{parse_exec_command, ExecCommand},
};

#[derive(Debug, Clone)]
pub struct AuthInfo {
    pub fingerprint: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy)]
pub struct PtySize {
    pub width: u16,
    pub height: u16,
}

impl Default for PtySize {
    fn default() -> Self {
        Self {
            width: 80,
            height: 24,
        }
    }
}

pub enum ChannelState {
    Tui {
        tui: Box<TuiResources>,
        audit: ActiveTuiSession,
    },
    Managed {
        control: BridgeControl,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveTuiSession {
    id: String,
}

impl ActiveTuiSession {
    pub(crate) fn new(id: String) -> Self {
        Self { id }
    }

    pub(crate) async fn finish(self, db: &HopDb, status: &str, error: Option<&str>) -> Result<()> {
        db.finish_session(&self.id, status, error).await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct HopSshServer {
    db: HopDb,
    config: HopConfig,
    master_key: Arc<MasterKey>,
}

pub async fn serve_ssh(
    bind: SocketAddr,
    config: HopConfig,
    db: HopDb,
    master_key: Arc<MasterKey>,
) -> Result<()> {
    let host_key =
        host_key::load_or_generate(&config.ssh.host_key_file, &config.ssh.host_key_type)?;
    let russh_config = server::Config {
        inactivity_timeout: Some(Duration::from_secs(3600)),
        keepalive_interval: ssh_keepalive_interval(&config),
        auth_rejection_time: Duration::from_secs(1),
        auth_rejection_time_initial: Some(Duration::from_millis(100)),
        keys: vec![host_key],
        nodelay: true,
        ..Default::default()
    };
    let mut server = HopSshServer {
        db,
        config,
        master_key,
    };
    let listener = TcpListener::bind(bind).await?;
    info!(%bind, "ssh server listening");
    server
        .run_on_socket(Arc::new(russh_config), &listener)
        .await?;
    Ok(())
}

fn ssh_keepalive_interval(config: &HopConfig) -> Option<Duration> {
    match config.ssh.keepalive_interval {
        0 => None,
        seconds => Some(Duration::from_secs(seconds)),
    }
}

impl server::Server for HopSshServer {
    type Handler = HopSshHandler;

    fn new_client(&mut self, peer_addr: Option<SocketAddr>) -> Self::Handler {
        HopSshHandler {
            db: self.db.clone(),
            config: self.config.clone(),
            master_key: self.master_key.clone(),
            auth: None,
            client_ip: peer_addr.map(|addr| addr.to_string()),
            channels: Arc::new(Mutex::new(HashMap::new())),
            ptys: HashMap::new(),
        }
    }

    fn handle_session_error(&mut self, error: <Self::Handler as server::Handler>::Error) {
        if is_client_disconnect(&error) {
            warn!(?error, "ssh client disconnected");
        } else {
            error!(?error, "ssh session error");
        }
    }
}

pub struct HopSshHandler {
    db: HopDb,
    config: HopConfig,
    master_key: Arc<MasterKey>,
    auth: Option<AuthInfo>,
    client_ip: Option<String>,
    channels: SharedChannels,
    ptys: HashMap<ChannelId, PtySize>,
}

impl HopSshHandler {
    fn auth_info(&self) -> Option<AuthInfo> {
        self.auth.clone()
    }

    fn pty_size(&self, channel: ChannelId) -> PtySize {
        self.ptys.get(&channel).copied().unwrap_or_default()
    }

    async fn start_tui_session(&self) -> Result<ActiveTuiSession> {
        let auth = self.auth_info().context("missing authenticated key")?;
        start_tui_session(&self.db, &auth, self.client_ip.clone()).await
    }

    async fn start_managed(
        &mut self,
        channel_id: ChannelId,
        handle: russh::server::Handle,
        asset: Asset,
        tui: Option<TuiResources>,
    ) -> Result<()> {
        let auth = self.auth_info().context("missing authenticated key")?;
        let options = ManagedBridgeOptions {
            db: self.db.clone(),
            master_key: self.master_key.clone(),
            auth,
            client_ip: self.client_ip.clone(),
            asset,
            channel_id,
            handle,
            pty: self.pty_size(channel_id),
            return_to_tui: tui.map(|tui| (self.channels.clone(), tui)),
            connect_timeout: self.config.connect_timeout(),
        };
        let control = bridge::spawn_managed_bridge(options);
        self.channels
            .lock()
            .await
            .insert(channel_id, ChannelState::Managed { control });
        Ok(())
    }
}

pub(crate) async fn start_tui_session(
    db: &HopDb,
    auth: &AuthInfo,
    client_ip: Option<String>,
) -> Result<ActiveTuiSession> {
    let session = db
        .start_session(NewSession {
            key_finger: auth.fingerprint.clone(),
            key_name: Some(auth.name.clone()),
            mode: "tui".to_string(),
            asset_name: None,
            target_host: None,
            target_port: None,
            client_ip,
        })
        .await?;
    Ok(ActiveTuiSession::new(session.id))
}

impl server::Handler for HopSshHandler {
    type Error = anyhow::Error;

    async fn authentication_banner(&mut self) -> Result<Option<String>, Self::Error> {
        Ok(authentication_banner(&self.config))
    }

    async fn auth_publickey_offered(
        &mut self,
        _user: &str,
        public_key: &PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        let fingerprint = key_fingerprint(public_key);
        if self
            .db
            .get_active_authorized_key_by_fingerprint(&fingerprint)
            .await?
            .is_some()
        {
            Ok(server::Auth::Accept)
        } else {
            Ok(server::Auth::reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        _user: &str,
        public_key: &PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        let fingerprint = key_fingerprint(public_key);
        match self
            .db
            .get_active_authorized_key_by_fingerprint(&fingerprint)
            .await?
        {
            Some(key) => {
                self.auth = Some(AuthInfo {
                    fingerprint,
                    name: key.name,
                });
                Ok(server::Auth::Accept)
            }
            None => Ok(server::Auth::reject()),
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let Some(auth) = self.auth_info() else {
            return Ok(false);
        };
        let target = self
            .db
            .find_proxy_asset(host_to_connect, i64::from(port_to_connect))
            .await?;
        let Some(asset) = target else {
            if let Ok(session) = self
                .db
                .start_session(NewSession {
                    key_finger: auth.fingerprint,
                    key_name: Some(auth.name),
                    mode: "proxyjump".to_string(),
                    asset_name: None,
                    target_host: Some(host_to_connect.to_string()),
                    target_port: Some(i64::from(port_to_connect)),
                    client_ip: self.client_ip.clone(),
                })
                .await
            {
                let _ = self
                    .db
                    .finish_session(
                        &session.id,
                        "failed",
                        Some("target not in assets allowlist"),
                    )
                    .await;
            }
            warn!(
                host_to_connect,
                port_to_connect, "rejected proxy target outside allowlist"
            );
            return Ok(false);
        };

        let db = self.db.clone();
        let client_ip = self.client_ip.clone();
        tokio::spawn(async move {
            if let Err(err) = proxy::bridge_direct_tcpip(channel, db, auth, asset, client_ip).await
            {
                warn!(?err, "proxy bridge failed");
            }
        });
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.ptys.insert(
            channel,
            PtySize {
                width: col_width as u16,
                height: row_height as u16,
            },
        );
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        let assets = self.db.list_assets().await?;
        let size = self.pty_size(channel);
        let mut tui = TuiResources::new(size.width, size.height, assets)?;
        send_tui_output(session, channel, tui.enter_screen()?)?;
        let audit = self.start_tui_session().await?;
        self.channels.lock().await.insert(
            channel,
            ChannelState::Tui {
                tui: Box::new(tui),
                audit,
            },
        );
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        let command = match parse_exec_command(data) {
            Ok(command) => command,
            Err(err) => {
                session.data(channel, format!("unsupported command: {err}\n"))?;
                session.exit_status_request(channel, 127)?;
                session.eof(channel)?;
                session.close(channel)?;
                return Ok(());
            }
        };

        match command {
            ExecCommand::Version => {
                session.data(
                    channel,
                    format!(
                        "{{\"name\":\"hop\",\"version\":\"{}\",\"protocol\":1}}\n",
                        env!("CARGO_PKG_VERSION")
                    ),
                )?;
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
                session.close(channel)?;
            }
            ExecCommand::ListAssets => {
                let assets = self.db.list_assets().await?;
                let payload = serde_json::to_string(&assets)?;
                session.data(channel, format!("{payload}\n"))?;
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
                session.close(channel)?;
            }
            ExecCommand::Connect { asset } => {
                let Some(asset) = self.db.get_asset_by_name(&asset).await? else {
                    session.data(channel, b"asset not found\n".to_vec())?;
                    session.exit_status_request(channel, 2)?;
                    session.eof(channel)?;
                    session.close(channel)?;
                    return Ok(());
                };
                if asset.credential_id.is_none() {
                    session.data(channel, b"asset has no managed credential\n".to_vec())?;
                    session.exit_status_request(channel, 3)?;
                    session.eof(channel)?;
                    session.close(channel)?;
                    return Ok(());
                }
                self.start_managed(channel, session.handle(), asset, None)
                    .await?;
            }
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let mut connect: Option<(Asset, TuiResources)> = None;
        let mut finish_tui: Option<(ActiveTuiSession, &'static str, Option<String>)> = None;
        let mut close = false;
        let mut output = Vec::new();
        {
            let mut channels = self.channels.lock().await;
            let action = if let Some(state) = channels.get_mut(&channel) {
                match state {
                    ChannelState::Tui { tui, .. } => {
                        let (action, mut rendered) = tui.handle_bytes(data)?;
                        output.append(&mut rendered);
                        Some(action)
                    }
                    ChannelState::Managed { control } => {
                        let _ = control.input.send(data.to_vec());
                        None
                    }
                }
            } else {
                None
            };

            match action {
                Some(TuiAction::None) | None => {}
                Some(TuiAction::Quit) => {
                    if let Some(ChannelState::Tui { mut tui, audit }) = channels.remove(&channel) {
                        output.append(&mut tui.leave_screen()?);
                        finish_tui = Some((audit, "ok", None));
                    }
                    close = true;
                }
                Some(TuiAction::Connect(asset)) => {
                    if asset.credential_id.is_none() {
                        output.extend_from_slice(
                            b"\r\nAsset has no managed credential; use ProxyJump instead.\r\n",
                        );
                    } else if let Some(ChannelState::Tui { mut tui, audit }) =
                        channels.remove(&channel)
                    {
                        output.append(&mut tui.leave_screen()?);
                        finish_tui = Some((audit, "connected", None));
                        connect = Some((asset, *tui));
                    }
                }
            }
        }
        send_tui_output(session, channel, output)?;
        if let Some((audit, status, error)) = finish_tui {
            let _ = audit.finish(&self.db, status, error.as_deref()).await;
        }
        if close {
            session.exit_status_request(channel, 0)?;
            session.eof(channel)?;
            session.close(channel)?;
        }
        if let Some((asset, tui)) = connect {
            self.start_managed(channel, session.handle(), asset, Some(tui))
                .await?;
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let mut finish_tui = None;
        {
            let mut channels = self.channels.lock().await;
            match channels.get(&channel) {
                Some(ChannelState::Managed { control }) => {
                    let _ = control.input.send(Vec::new());
                }
                Some(ChannelState::Tui { .. }) => {
                    if let Some(ChannelState::Tui { mut tui, audit }) = channels.remove(&channel) {
                        if let Ok(output) = tui.leave_screen() {
                            let _ = send_tui_output(session, channel, output);
                        }
                        finish_tui = Some(audit);
                    }
                }
                None => {}
            }
        }
        if let Some(audit) = finish_tui {
            let _ = audit.finish(&self.db, "ok", None).await;
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let state = self.channels.lock().await.remove(&channel);
        if let Some(ChannelState::Tui { mut tui, audit }) = state {
            if let Ok(output) = tui.leave_screen() {
                let _ = send_tui_output(session, channel, output);
            }
            let _ = audit.finish(&self.db, "ok", None).await;
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let size = PtySize {
            width: col_width as u16,
            height: row_height as u16,
        };
        self.ptys.insert(channel, size);
        let mut channels = self.channels.lock().await;
        if let Some(state) = channels.get_mut(&channel) {
            match state {
                ChannelState::Tui { tui, .. } => {
                    let output = tui.resize(size.width, size.height)?;
                    send_tui_output(session, channel, output)?;
                }
                ChannelState::Managed { control } => {
                    let _ = control.resize.send(size);
                }
            }
        }
        Ok(())
    }
}

fn send_tui_output(session: &mut Session, channel: ChannelId, output: Vec<u8>) -> Result<()> {
    if !output.is_empty() {
        session.data(channel, output)?;
    }
    Ok(())
}

fn key_fingerprint(key: &PublicKey) -> String {
    format!("{}", key.fingerprint(HashAlg::Sha256))
}

fn authentication_banner(config: &HopConfig) -> Option<String> {
    let banner = &config.ssh.banner;
    if banner.is_empty() {
        return None;
    }
    if banner.ends_with('\n') {
        Some(banner.clone())
    } else {
        Some(format!("{banner}\r\n"))
    }
}

fn is_client_disconnect(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<io::Error>()
        .map(|err| {
            matches!(
                err.kind(),
                io::ErrorKind::ConnectionReset
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::UnexpectedEof
            ) || matches!(err.raw_os_error(), Some(10054) | Some(104))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use hop_core::NewSession;

    use super::*;

    #[tokio::test]
    async fn active_tui_session_finish_marks_record_done() {
        let db = HopDb::in_memory().await.unwrap();
        let session = db
            .start_session(NewSession {
                key_finger: "SHA256:test".to_string(),
                key_name: Some("tester".to_string()),
                mode: "tui".to_string(),
                asset_name: None,
                target_host: None,
                target_port: None,
                client_ip: None,
            })
            .await
            .unwrap();

        ActiveTuiSession::new(session.id.clone())
            .finish(&db, "ok", None)
            .await
            .unwrap();

        let finished = db.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(finished.status, "ok");
        assert!(finished.ended_at.is_some());
    }

    #[test]
    fn ssh_keepalive_interval_uses_config_value() {
        let mut config = HopConfig::default();
        assert_eq!(
            ssh_keepalive_interval(&config),
            Some(Duration::from_secs(30))
        );

        config.ssh.keepalive_interval = 0;
        assert_eq!(ssh_keepalive_interval(&config), None);
    }

    #[test]
    fn authentication_banner_adds_newline_for_openssh_prompts() {
        let mut config = HopConfig::default();
        config.ssh.banner = "Welcome to Hop".to_string();

        assert_eq!(
            authentication_banner(&config).as_deref(),
            Some("Welcome to Hop\r\n")
        );
    }

    #[test]
    fn empty_authentication_banner_is_disabled() {
        let mut config = HopConfig::default();
        config.ssh.banner = String::new();

        assert!(authentication_banner(&config).is_none());
    }

    #[test]
    fn connection_reset_is_treated_as_client_disconnect() {
        let error = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionReset));

        assert!(is_client_disconnect(&error));
    }
}
