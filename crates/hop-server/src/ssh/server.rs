use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

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
    host_key,
    proxy,
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
        Self { width: 80, height: 24 }
    }
}

pub enum ChannelState {
    Tui(TuiResources),
    Managed { control: BridgeControl },
}

#[derive(Clone)]
pub struct HopSshServer {
    db: HopDb,
    config: HopConfig,
    master_key: Arc<MasterKey>,
}

pub async fn serve_ssh(bind: SocketAddr, config: HopConfig, db: HopDb, master_key: Arc<MasterKey>) -> Result<()> {
    let host_key = host_key::load_or_generate(&config.ssh.host_key_file, &config.ssh.host_key_type)?;
    let russh_config = server::Config {
        inactivity_timeout: Some(Duration::from_secs(3600)),
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
    server.run_on_socket(Arc::new(russh_config), &listener).await?;
    Ok(())
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
        error!(?error, "ssh session error");
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

impl server::Handler for HopSshHandler {
    type Error = anyhow::Error;

    async fn authentication_banner(&mut self) -> Result<Option<String>, Self::Error> {
        Ok(Some(self.config.ssh.banner.clone()))
    }

    async fn auth_publickey_offered(&mut self, _user: &str, public_key: &PublicKey) -> Result<server::Auth, Self::Error> {
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

    async fn auth_publickey(&mut self, _user: &str, public_key: &PublicKey) -> Result<server::Auth, Self::Error> {
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

    async fn channel_open_session(&mut self, _channel: Channel<Msg>, _session: &mut Session) -> Result<bool, Self::Error> {
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
                    .finish_session(&session.id, "failed", Some("target not in assets allowlist"))
                    .await;
            }
            warn!(host_to_connect, port_to_connect, "rejected proxy target outside allowlist");
            return Ok(false);
        };

        let db = self.db.clone();
        let client_ip = self.client_ip.clone();
        tokio::spawn(async move {
            if let Err(err) = proxy::bridge_direct_tcpip(channel, db, auth, asset, client_ip).await {
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

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        let assets = self.db.list_assets().await?;
        let size = self.pty_size(channel);
        let mut tui = TuiResources::new(session.handle(), channel, size.width, size.height, assets)?;
        tui.render()?;
        self.channels.lock().await.insert(channel, ChannelState::Tui(tui));
        self.db
            .start_session(NewSession {
                key_finger: self.auth.as_ref().map(|a| a.fingerprint.clone()).unwrap_or_default(),
                key_name: self.auth.as_ref().map(|a| a.name.clone()),
                mode: "tui".to_string(),
                asset_name: None,
                target_host: None,
                target_port: None,
                client_ip: self.client_ip.clone(),
            })
            .await
            .ok();
        Ok(())
    }

    async fn exec_request(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
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
                self.start_managed(channel, session.handle(), asset, None).await?;
            }
        }
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        let mut connect: Option<(Asset, TuiResources)> = None;
        let mut close = false;
        {
            let mut channels = self.channels.lock().await;
            if let Some(state) = channels.get_mut(&channel) {
                match state {
                    ChannelState::Tui(tui) => match tui.handle_bytes(data)? {
                        TuiAction::None => {}
                        TuiAction::Quit => close = true,
                        TuiAction::Connect(asset) => {
                            if asset.credential_id.is_none() {
                                let _ = session.data(channel, b"\r\nAsset has no managed credential; use ProxyJump instead.\r\n".to_vec());
                            } else if let Some(ChannelState::Tui(tui)) = channels.remove(&channel) {
                                connect = Some((asset, tui));
                            }
                        }
                    },
                    ChannelState::Managed { control } => {
                        let _ = control.input.send(data.to_vec());
                    }
                }
            }
        }
        if close {
            session.eof(channel)?;
            session.close(channel)?;
        }
        if let Some((asset, tui)) = connect {
            self.start_managed(channel, session.handle(), asset, Some(tui)).await?;
        }
        Ok(())
    }

    async fn channel_eof(&mut self, channel: ChannelId, _session: &mut Session) -> Result<(), Self::Error> {
        if let Some(ChannelState::Managed { control }) = self.channels.lock().await.get(&channel) {
            let _ = control.input.send(Vec::new());
        }
        Ok(())
    }

    async fn channel_close(&mut self, channel: ChannelId, _session: &mut Session) -> Result<(), Self::Error> {
        self.channels.lock().await.remove(&channel);
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let size = PtySize {
            width: col_width as u16,
            height: row_height as u16,
        };
        self.ptys.insert(channel, size);
        let mut channels = self.channels.lock().await;
        if let Some(state) = channels.get_mut(&channel) {
            match state {
                ChannelState::Tui(tui) => {
                    tui.resize(size.width, size.height)?;
                }
                ChannelState::Managed { control } => {
                    let _ = control.resize.send(size);
                }
            }
        }
        Ok(())
    }
}

fn key_fingerprint(key: &PublicKey) -> String {
    format!("{}", key.fingerprint(HashAlg::Sha256))
}
