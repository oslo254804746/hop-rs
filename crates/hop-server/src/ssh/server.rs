use std::{
    collections::HashMap,
    io,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use hop_core::{Asset, Catalog, HopConfig, MasterKey, NewAuditEvent, NewSession};
use russh::{
    keys::{ssh_key::HashAlg, PublicKey},
    server::{self, Msg, Session},
    Channel, ChannelId, ChannelOpenFailure, MethodKind, MethodSet, Pty,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch, Mutex},
    task::JoinSet,
};
use tracing::{error, info, warn};

use crate::tui::{TuiAction, TuiResources};

use super::{
    bridge::{
        self, BridgeControl, BridgeInput, DeferredSftpBridgeOptions, ManagedBridgeOptions,
        ManagedSessionMode, SharedChannels,
    },
    host_key, proxy,
    session_registry::{
        ActiveSessionRegistry, TuiSelectorGuard, TuiTargetGuard, TERMINATED_BY_MANAGEMENT,
    },
};

#[derive(Debug, Clone)]
pub struct AuthInfo {
    pub key_id: String,
    pub fingerprint: String,
    pub name: String,
}

pub(crate) const AUTHORIZATION_DENIED: &str = "target not authorized";
const TUI_SFTP_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DenialReason {
    TargetNotAuthorized,
    TargetNotFound,
    ForwardingDisabledForAsset,
    ForwardingDisabledByServer,
    TargetNotManagedSsh,
}

impl DenialReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TargetNotAuthorized => "target not authorized",
            Self::TargetNotFound => "target not found",
            Self::ForwardingDisabledForAsset => "tcp forwarding disabled for asset",
            Self::ForwardingDisabledByServer => "managed tcp forwarding disabled by server policy",
            Self::TargetNotManagedSsh => "target does not support managed ssh forwarding",
        }
    }

    pub(crate) fn audit_action(self, mode: &str) -> &'static str {
        match self {
            Self::ForwardingDisabledForAsset
            | Self::ForwardingDisabledByServer
            | Self::TargetNotManagedSsh => "session.forward_denied",
            Self::TargetNotFound if matches!(mode, "managed-tcp-forward" | "tcp-forward") => {
                "session.forward_denied"
            }
            Self::TargetNotAuthorized | Self::TargetNotFound => "session.access_denied",
        }
    }
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
        #[expect(
            dead_code,
            reason = "RAII guard dropped when channel leaves TUI selector state"
        )]
        selector_guard: TuiSelectorGuard,
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

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) async fn finish(
        self,
        db: &Catalog,
        status: &str,
        error: Option<&str>,
    ) -> Result<()> {
        db.finish_session(&self.id, status, error).await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct HopSshServer {
    db: Catalog,
    config: HopConfig,
    master_key: Arc<MasterKey>,
    active_sessions: ActiveSessionRegistry,
}

pub async fn serve_ssh_with_shutdown(
    bind: SocketAddr,
    config: HopConfig,
    db: Catalog,
    master_key: Arc<MasterKey>,
    active_sessions: ActiveSessionRegistry,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let host_key =
        host_key::load_or_generate(&config.ssh_host_key_path(), &config.ssh.host_key_type)?;
    let russh_config = server::Config {
        methods: ingress_auth_methods(),
        inactivity_timeout: Some(Duration::from_secs(3600)),
        keepalive_interval: ssh_keepalive_interval(&config),
        auth_rejection_time: Duration::from_secs(1),
        auth_rejection_time_initial: Some(Duration::from_millis(100)),
        keys: vec![host_key],
        nodelay: true,
        ..Default::default()
    };
    let server = HopSshServer {
        db,
        config,
        master_key,
        active_sessions,
    };
    let listener = TcpListener::bind(bind).await?;
    info!(%bind, "ssh server listening");
    serve_ssh_connections(listener, Arc::new(russh_config), server, &mut shutdown).await
}

async fn serve_ssh_connections(
    listener: TcpListener,
    config: Arc<server::Config>,
    server: HopSshServer,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<()> {
    let mut sessions = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (socket, client_ip) = accepted?;
                if !server.config.ssh.allows_client_ip(client_ip.ip()) {
                    warn!(%client_ip, "rejected SSH client outside allowed IP ranges");
                    continue;
                }
                if config.nodelay {
                    if let Err(error) = socket.set_nodelay(true) {
                        warn!(%client_ip, ?error, "failed to enable TCP_NODELAY for SSH client");
                    }
                }
                let handler = server.new_client(client_ip);
                let session_config = Arc::clone(&config);
                sessions.spawn(async move {
                    let result = match server::run_stream(session_config, socket, handler).await {
                        Ok(session) => session.await,
                        Err(error) => Err(error),
                    };
                    (client_ip, result)
                });
            }
            Some(result) = sessions.join_next(), if !sessions.is_empty() => {
                match result {
                    Ok((client_ip, Ok(()))) => {
                        tracing::debug!(%client_ip, "ssh client disconnected");
                    }
                    Ok((client_ip, Err(error))) => {
                        log_ssh_session_error(client_ip, &error);
                    }
                    Err(error) => {
                        error!(?error, "ssh session task failed");
                    }
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    let signaled = server.active_sessions.terminate_all().await;
                    info!(signaled, "SSH shutdown requested; draining existing connections");
                    drain_ssh_sessions(&mut sessions).await;
                    return Ok(());
                }
            }
        }
    }
}

const SSH_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

async fn drain_ssh_sessions(sessions: &mut JoinSet<(SocketAddr, Result<()>)>) {
    let drain = async {
        while let Some(result) = sessions.join_next().await {
            match result {
                Ok((client_ip, Ok(()))) => {
                    tracing::debug!(%client_ip, "ssh client disconnected during shutdown");
                }
                Ok((client_ip, Err(error))) => {
                    log_ssh_session_error(client_ip, &error);
                }
                Err(error) => {
                    error!(?error, "ssh session task failed during shutdown");
                }
            }
        }
    };

    if tokio::time::timeout(SSH_DRAIN_TIMEOUT, drain)
        .await
        .is_err()
    {
        warn!(
            timeout_seconds = SSH_DRAIN_TIMEOUT.as_secs(),
            remaining = sessions.len(),
            "SSH connection drain timed out; terminating remaining connections"
        );
        sessions.abort_all();
        while sessions.join_next().await.is_some() {}
    }
}

fn ingress_auth_methods() -> MethodSet {
    MethodSet::from(&[MethodKind::PublicKey][..])
}

fn ssh_keepalive_interval(config: &HopConfig) -> Option<Duration> {
    match config.ssh.keepalive_interval {
        0 => None,
        seconds => Some(Duration::from_secs(seconds)),
    }
}

impl HopSshServer {
    fn new_client(&self, peer_addr: SocketAddr) -> HopSshHandler {
        HopSshHandler {
            db: self.db.clone(),
            config: self.config.clone(),
            master_key: self.master_key.clone(),
            active_sessions: self.active_sessions.clone(),
            auth: None,
            direct_asset: None,
            selected_asset: Arc::new(StdMutex::new(None)),
            direct_client: Arc::new(Mutex::new(None)),
            peer_ip: Some(peer_addr.ip().to_canonical()),
            client_ip: Some(peer_addr.to_string()),
            channels: Arc::new(Mutex::new(HashMap::new())),
            ptys: HashMap::new(),
            pty_terms: HashMap::new(),
        }
    }
}

pub struct HopSshHandler {
    db: Catalog,
    config: HopConfig,
    master_key: Arc<MasterKey>,
    active_sessions: ActiveSessionRegistry,
    auth: Option<AuthInfo>,
    direct_asset: Option<Asset>,
    selected_asset: Arc<StdMutex<Option<Asset>>>,
    direct_client: Arc<Mutex<Option<Arc<super::client::ManagedClientSession>>>>,
    peer_ip: Option<IpAddr>,
    client_ip: Option<String>,
    channels: SharedChannels,
    ptys: HashMap<ChannelId, PtySize>,
    pty_terms: HashMap<ChannelId, String>,
}

impl HopSshHandler {
    fn auth_info(&self) -> Option<AuthInfo> {
        self.auth.clone()
    }

    fn effective_managed_asset(&self) -> Option<Asset> {
        self.direct_asset.clone().or_else(|| {
            self.selected_asset
                .lock()
                .expect("selected_asset mutex poisoned")
                .clone()
        })
    }

    fn pty_size(&self, channel: ChannelId) -> PtySize {
        self.ptys.get(&channel).copied().unwrap_or_default()
    }

    fn pty_term(&self, channel: ChannelId) -> String {
        self.pty_terms
            .get(&channel)
            .cloned()
            .unwrap_or_else(|| "xterm-256color".to_string())
    }

    async fn start_tui_session(&self) -> Result<ActiveTuiSession> {
        let auth = self.auth_info().context("missing authenticated key")?;
        start_tui_session(&self.db, &auth, self.client_ip.clone()).await
    }

    async fn resolve_direct_asset_for_key(
        &self,
        user: &str,
        key_id: &str,
    ) -> Result<Option<Asset>> {
        let Some(request) = parse_direct_username(user)? else {
            return Ok(None);
        };
        let Some(asset) = self
            .db
            .find_direct_asset_for_key(key_id, &request.target)
            .await?
        else {
            return Ok(None);
        };
        if !accepts_managed_ssh_asset(Some(&asset)) {
            return Ok(None);
        }
        Ok(Some(asset))
    }

    async fn resolve_channel_managed_asset(&self, wait_timeout: Duration) -> Result<Option<Asset>> {
        if let Some(asset) = self.direct_asset.clone() {
            return Ok(Some(asset));
        }
        let selected = self
            .selected_asset
            .lock()
            .expect("selected_asset mutex poisoned")
            .clone();
        if let Some(selected) = selected {
            return self
                .db
                .get_asset_by_id(&selected.id)
                .await
                .map_err(Into::into);
        }
        let Some(auth) = self.auth_info() else {
            return Ok(None);
        };
        let Some(candidate) = self
            .active_sessions
            .resolve_tui_target(&auth.key_id, self.peer_ip, wait_timeout)
            .await
        else {
            return Ok(None);
        };
        self.db
            .get_asset_by_id(&candidate.id)
            .await
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_managed(
        &mut self,
        channel_id: ChannelId,
        handle: russh::server::Handle,
        asset: Asset,
        tui: Option<TuiResources>,
        tui_target_guard: Option<TuiTargetGuard>,
        mode: ManagedSessionMode,
        command: Option<Vec<u8>>,
    ) -> Result<()> {
        let auth = self.auth_info().context("missing authenticated key")?;
        if !managed_mode_is_authorized(&self.db, &auth.key_id, &asset.id, mode).await? {
            audit_denied_attempt(
                &self.db,
                &auth,
                managed_session_mode_name(mode),
                DenialReason::TargetNotAuthorized,
                Some(asset.id.clone()),
                Some(asset.name.clone()),
                Some(asset.hostname.clone()),
                Some(asset.port),
                self.client_ip.clone(),
            )
            .await;
            bail!(AUTHORIZATION_DENIED);
        }
        let selected_asset_cleanup = (mode == ManagedSessionMode::Tui).then(|| {
            (
                Arc::clone(&self.selected_asset),
                Arc::clone(&self.direct_client),
            )
        });
        let options = ManagedBridgeOptions {
            db: self.db.clone(),
            master_key: self.master_key.clone(),
            active_sessions: self.active_sessions.clone(),
            auth,
            peer_ip: self.peer_ip,
            client_ip: self.client_ip.clone(),
            asset,
            channel_id,
            handle,
            pty: self.pty_size(channel_id),
            pty_term: self.pty_term(channel_id),
            pty_requested: self.ptys.contains_key(&channel_id),
            command,
            session_mode: mode,
            return_to_tui: tui.map(|tui| (self.channels.clone(), tui)),
            tui_target_guard,
            selected_asset_cleanup,
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

async fn managed_mode_is_authorized(
    db: &Catalog,
    key_id: &str,
    asset_id: &str,
    _mode: ManagedSessionMode,
) -> Result<bool> {
    db.key_can_access_asset(key_id, asset_id)
        .await
        .map_err(Into::into)
}

fn managed_session_mode_name(mode: ManagedSessionMode) -> &'static str {
    match mode {
        ManagedSessionMode::Tui => "tui-connect",
        ManagedSessionMode::Direct => "direct",
        ManagedSessionMode::Exec => "exec",
        ManagedSessionMode::Sftp => "sftp",
    }
}

async fn classify_proxy_denial(
    db: &Catalog,
    host_to_connect: &str,
    port: i64,
) -> (DenialReason, Option<Asset>) {
    let normalized_name = host_to_connect
        .strip_suffix(".hop")
        .unwrap_or(host_to_connect);
    if let Ok(assets) = db.list_assets().await {
        if let Some(found) = assets.into_iter().find(|asset| {
            asset.port == port
                && (asset.hostname == host_to_connect || asset.name == normalized_name)
        }) {
            return (DenialReason::TargetNotAuthorized, Some(found));
        }
    }
    (DenialReason::TargetNotFound, None)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn audit_denied_attempt(
    db: &Catalog,
    auth: &AuthInfo,
    mode: &str,
    reason: DenialReason,
    asset_id: Option<String>,
    asset_name: Option<String>,
    target_host: Option<String>,
    target_port: Option<i64>,
    client_ip: Option<String>,
) {
    let target_endpoint = match (&target_host, target_port) {
        (Some(host), Some(port)) => Some(format!("{host}:{port}")),
        (Some(host), None) => Some(host.clone()),
        _ => None,
    };
    let target_label = asset_name.clone().or_else(|| target_endpoint.clone());
    let mut session_id = None;
    if let Ok(session) = db
        .start_session(NewSession {
            key_id: Some(auth.key_id.clone()),
            key_finger: auth.fingerprint.clone(),
            key_name: Some(auth.name.clone()),
            mode: mode.to_string(),
            asset_id,
            asset_name,
            target_host,
            target_port,
            client_ip: client_ip.clone(),
        })
        .await
    {
        let _ = db
            .finish_session(&session.id, "failed", Some(reason.as_str()))
            .await;
        session_id = Some(session.id);
    }

    let _ = db
        .record_audit_event(NewAuditEvent {
            actor_label: "ssh-ingress".to_string(),
            action: reason.audit_action(mode).to_string(),
            target_type: "session".to_string(),
            target_id: session_id,
            target_label,
            result: "denied".to_string(),
            source_ip: client_ip,
            details_json: Some(
                serde_json::json!({
                    "key_name": auth.name,
                    "mode": mode,
                    "reason": reason.as_str(),
                    "target": target_endpoint,
                })
                .to_string(),
            ),
        })
        .await;
}

pub(crate) async fn start_tui_session(
    db: &Catalog,
    auth: &AuthInfo,
    client_ip: Option<String>,
) -> Result<ActiveTuiSession> {
    let session = db
        .start_session(NewSession {
            key_id: Some(auth.key_id.clone()),
            key_finger: auth.fingerprint.clone(),
            key_name: Some(auth.name.clone()),
            mode: "tui".to_string(),
            asset_id: None,
            asset_name: None,
            target_host: None,
            target_port: None,
            client_ip,
        })
        .await?;
    Ok(ActiveTuiSession::new(session.id))
}

pub(crate) fn spawn_tui_termination_watcher(
    db: Catalog,
    active_sessions: ActiveSessionRegistry,
    session_id: String,
    channels: SharedChannels,
    channel_id: ChannelId,
    handle: russh::server::Handle,
    mut terminate: mpsc::UnboundedReceiver<()>,
) {
    tokio::spawn(async move {
        let Some(()) = terminate.recv().await else {
            return;
        };

        let terminated = {
            let mut channels = channels.lock().await;
            match channels.remove(&channel_id) {
                Some(ChannelState::Tui { mut tui, audit, .. }) => {
                    let mut output = tui.leave_screen().unwrap_or_default();
                    output.extend_from_slice(
                        format!("\r\n{TERMINATED_BY_MANAGEMENT}\r\n").as_bytes(),
                    );
                    Some((audit, output))
                }
                Some(state) => {
                    channels.insert(channel_id, state);
                    None
                }
                None => None,
            }
        };

        if let Some((audit, output)) = terminated {
            if !output.is_empty() {
                let _ = handle.data(channel_id, output).await;
            }
            let _ = audit
                .finish(&db, "terminated", Some(TERMINATED_BY_MANAGEMENT))
                .await;
            let _ = handle.exit_status_request(channel_id, 255).await;
            let _ = handle.eof(channel_id).await;
            let _ = handle.close(channel_id).await;
        }
        active_sessions.unregister(&session_id).await;
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectLoginRequest {
    target: String,
}

fn parse_direct_username(user: &str) -> Result<Option<DirectLoginRequest>> {
    let target = user.trim();
    if target.is_empty() {
        bail!("direct username requires <asset>");
    }
    if target.contains('@') {
        bail!("direct username must be a single asset name or hostname");
    }
    if target.contains(char::is_whitespace) {
        bail!("direct username cannot contain whitespace");
    }
    if target.eq_ignore_ascii_case("menu") || target.eq_ignore_ascii_case("+tui") {
        return Ok(None);
    }
    Ok(Some(DirectLoginRequest {
        target: target.to_string(),
    }))
}

impl server::Handler for HopSshHandler {
    type Error = anyhow::Error;

    async fn authentication_banner(&mut self) -> Result<Option<String>, Self::Error> {
        Ok(authentication_banner(&self.config))
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        let fingerprint = key_fingerprint(public_key);
        let Some(key) = self
            .db
            .get_active_authorized_key_by_fingerprint(&fingerprint)
            .await?
        else {
            warn!(
                client_ip = %self.client_ip.as_deref().unwrap_or("unknown"),
                user,
                fingerprint,
                "rejected unregistered ingress public key"
            );
            return Ok(server::Auth::reject());
        };
        if let Err(error) = self.resolve_direct_asset_for_key(user, &key.id).await {
            warn!(
                client_ip = %self.client_ip.as_deref().unwrap_or("unknown"),
                user,
                key_id = key.id,
                key_name = key.name,
                ?error,
                "rejected direct login request"
            );
            return Ok(server::Auth::reject());
        }
        Ok(server::Auth::Accept)
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        let fingerprint = key_fingerprint(public_key);
        match self
            .db
            .get_active_authorized_key_by_fingerprint(&fingerprint)
            .await?
        {
            Some(key) => {
                let direct_asset = match self.resolve_direct_asset_for_key(user, &key.id).await {
                    Ok(asset) => asset,
                    Err(err) => {
                        warn!(
                            client_ip = %self.client_ip.as_deref().unwrap_or("unknown"),
                            user,
                            key_id = key.id,
                            key_name = key.name,
                            ?err,
                            "rejected direct login request"
                        );
                        return Ok(server::Auth::reject());
                    }
                };
                info!(
                    client_ip = %self.client_ip.as_deref().unwrap_or("unknown"),
                    user,
                    key_id = key.id,
                    key_name = key.name,
                    fingerprint,
                    target = direct_asset.as_ref().map(|asset| asset.name.as_str()).unwrap_or("tui"),
                    "ssh ingress authenticated"
                );
                self.auth = Some(AuthInfo {
                    key_id: key.id,
                    fingerprint,
                    name: key.name,
                });
                self.direct_asset = direct_asset;
                Ok(server::Auth::Accept)
            }
            None => Ok(server::Auth::reject()),
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        originator_address: &str,
        originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(auth) = self.auth_info() else {
            reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        };

        let is_dot_hop = host_to_connect
            .trim()
            .to_ascii_lowercase()
            .ends_with(".hop");
        if let Some(direct) = self.effective_managed_asset().filter(|_| !is_dot_hop) {
            let fresh_asset = self.db.get_asset_by_id(&direct.id).await?;
            let Some(asset) = fresh_asset
                .clone()
                .filter(|a| accepts_managed_ssh_asset(Some(a)))
            else {
                let reason = if fresh_asset.is_none() {
                    DenialReason::TargetNotFound
                } else {
                    DenialReason::TargetNotManagedSsh
                };
                audit_denied_attempt(
                    &self.db,
                    &auth,
                    "managed-tcp-forward",
                    reason,
                    Some(direct.id.clone()),
                    Some(direct.name.clone()),
                    Some(host_to_connect.to_string()),
                    Some(i64::from(port_to_connect)),
                    self.client_ip.clone(),
                )
                .await;
                reply
                    .reject(ChannelOpenFailure::AdministrativelyProhibited)
                    .await;
                return Ok(());
            };

            let key_authorized = self
                .db
                .key_can_access_asset(&auth.key_id, &asset.id)
                .await?;
            let forwarding_enabled =
                self.config.ssh.allow_managed_tcp_forwarding && asset.allow_tcp_forwarding;
            if !key_authorized || !forwarding_enabled {
                let reason = if !key_authorized {
                    DenialReason::TargetNotAuthorized
                } else if !self.config.ssh.allow_managed_tcp_forwarding {
                    DenialReason::ForwardingDisabledByServer
                } else {
                    DenialReason::ForwardingDisabledForAsset
                };
                audit_denied_attempt(
                    &self.db,
                    &auth,
                    "managed-tcp-forward",
                    reason,
                    Some(asset.id.clone()),
                    Some(asset.name.clone()),
                    Some(host_to_connect.to_string()),
                    Some(i64::from(port_to_connect)),
                    self.client_ip.clone(),
                )
                .await;
                warn!(
                    key_id = auth.key_id,
                    asset = asset.name,
                    host_to_connect,
                    port_to_connect,
                    reason = reason.as_str(),
                    client_ip = ?self.client_ip,
                    "rejected managed tcp forwarding request"
                );
                reply
                    .reject(ChannelOpenFailure::AdministrativelyProhibited)
                    .await;
                return Ok(());
            }

            let direct_client = Arc::clone(&self.direct_client);
            let db = self.db.clone();
            let master_key = Arc::clone(&self.master_key);
            let active_sessions = self.active_sessions.clone();
            let client_ip = self.client_ip.clone();
            let connect_timeout = self.config.connect_timeout();
            let host = host_to_connect.to_string();
            let orig_addr = originator_address.to_string();
            tokio::spawn(async move {
                if let Err(err) = proxy::bridge_managed_direct_tcpip(
                    channel,
                    reply,
                    direct_client,
                    db,
                    master_key,
                    active_sessions,
                    auth,
                    asset,
                    host,
                    port_to_connect,
                    orig_addr,
                    originator_port,
                    client_ip,
                    connect_timeout,
                )
                .await
                {
                    tracing::debug!(?err, "managed tcp forward channel ended with error");
                }
            });
            return Ok(());
        }

        let target = self
            .db
            .find_proxy_asset_for_key(&auth.key_id, host_to_connect, i64::from(port_to_connect))
            .await?;
        let Some(asset) = target else {
            let (reason, matched) =
                classify_proxy_denial(&self.db, host_to_connect, i64::from(port_to_connect)).await;
            audit_denied_attempt(
                &self.db,
                &auth,
                "tcp-forward",
                reason,
                matched.as_ref().map(|a| a.id.clone()),
                matched.as_ref().map(|a| a.name.clone()),
                Some(host_to_connect.to_string()),
                Some(i64::from(port_to_connect)),
                self.client_ip.clone(),
            )
            .await;
            warn!(
                key_id = auth.key_id,
                host_to_connect,
                port_to_connect,
                reason = reason.as_str(),
                client_ip = ?self.client_ip,
                "rejected proxy target"
            );
            reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        };

        reply.accept().await;
        let db = self.db.clone();
        let master_key = Arc::clone(&self.master_key);
        let active_sessions = self.active_sessions.clone();
        let client_ip = self.client_ip.clone();
        let connect_timeout = self.config.connect_timeout();
        tokio::spawn(async move {
            if let Err(err) = proxy::bridge_direct_tcpip(
                channel,
                db,
                master_key,
                active_sessions,
                auth,
                asset,
                client_ip,
                connect_timeout,
            )
            .await
            {
                warn!(?err, "proxy bridge failed");
            }
        });
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
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
        self.pty_terms
            .insert(channel, super::client::sanitize_pty_term(term));
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        if let Some(asset) = self.direct_asset.clone() {
            self.start_managed(
                channel,
                session.handle(),
                asset,
                None,
                None,
                ManagedSessionMode::Direct,
                None,
            )
            .await?;
            return Ok(());
        }
        let auth = self.auth_info().context("missing authenticated key")?;
        let assets = self.db.list_assets_for_key(&auth.key_id).await?;
        let size = self.pty_size(channel);
        let mut tui = TuiResources::new(size.width, size.height, assets)?;
        send_tui_output(session, channel, tui.enter_screen()?)?;
        let audit = self.start_tui_session().await?;
        let session_id = audit.id().to_string();
        let selector_guard = self
            .active_sessions
            .register_tui_selector(&auth.key_id, self.peer_ip);
        let (terminate_tx, terminate_rx) = mpsc::unbounded_channel();
        self.channels.lock().await.insert(
            channel,
            ChannelState::Tui {
                tui: Box::new(tui),
                audit,
                selector_guard,
            },
        );
        spawn_tui_termination_watcher(
            self.db.clone(),
            self.active_sessions.clone(),
            session_id.clone(),
            self.channels.clone(),
            channel,
            session.handle(),
            terminate_rx,
        );
        self.active_sessions
            .register(session_id, terminate_tx)
            .await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let has_local_tui = self
            .channels
            .lock()
            .await
            .values()
            .any(|state| matches!(state, ChannelState::Tui { .. }));
        let wait_timeout = if has_local_tui {
            Duration::ZERO
        } else {
            TUI_SFTP_WAIT_TIMEOUT
        };
        let Some(asset) = self.resolve_channel_managed_asset(wait_timeout).await? else {
            session.channel_failure(channel)?;
            return Ok(());
        };
        if !accepts_managed_ssh_asset(Some(&asset)) {
            session.channel_failure(channel)?;
            return Ok(());
        }

        if self
            .start_managed(
                channel,
                session.handle(),
                asset,
                None,
                None,
                ManagedSessionMode::Exec,
                Some(data.to_vec()),
            )
            .await
            .is_err()
        {
            session.channel_failure(channel)?;
            return Ok(());
        }
        session.channel_success(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name != "sftp" {
            session.channel_failure(channel)?;
            return Ok(());
        }

        let has_local_tui = self
            .channels
            .lock()
            .await
            .values()
            .any(|state| matches!(state, ChannelState::Tui { .. }));
        if self.effective_managed_asset().is_none() && has_local_tui {
            let Some(auth) = self.auth_info() else {
                session.channel_failure(channel)?;
                return Ok(());
            };
            session.channel_success(channel)?;
            let control = bridge::spawn_deferred_sftp_bridge(DeferredSftpBridgeOptions {
                db: self.db.clone(),
                master_key: Arc::clone(&self.master_key),
                active_sessions: self.active_sessions.clone(),
                auth,
                peer_ip: self.peer_ip,
                client_ip: self.client_ip.clone(),
                channel_id: channel,
                handle: session.handle(),
                channels: Arc::clone(&self.channels),
                pty: self.pty_size(channel),
                pty_term: self.pty_term(channel),
                connect_timeout: self.config.connect_timeout(),
                wait_timeout: TUI_SFTP_WAIT_TIMEOUT,
            });
            self.channels
                .lock()
                .await
                .insert(channel, ChannelState::Managed { control });
            return Ok(());
        }

        let Some(asset) = self
            .resolve_channel_managed_asset(TUI_SFTP_WAIT_TIMEOUT)
            .await?
        else {
            session.channel_failure(channel)?;
            return Ok(());
        };
        if !accepts_sftp_request(name, Some(&asset)) {
            session.channel_failure(channel)?;
            return Ok(());
        }

        if self
            .start_managed(
                channel,
                session.handle(),
                asset,
                None,
                None,
                ManagedSessionMode::Sftp,
                None,
            )
            .await
            .is_err()
        {
            session.channel_failure(channel)?;
            return Ok(());
        }
        session.channel_success(channel)?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let mut connect: Option<(Asset, TuiResources, Option<TuiTargetGuard>)> = None;
        let mut finish_tui: Option<(ActiveTuiSession, &'static str, Option<String>)> = None;
        let mut probe_tui = false;
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
                        let _ = control.input.send(BridgeInput::Data(data.to_vec()));
                        None
                    }
                }
            } else {
                None
            };

            match action {
                Some(TuiAction::None) | None => {}
                Some(TuiAction::Quit) => {
                    if let Some(ChannelState::Tui { mut tui, audit, .. }) =
                        channels.remove(&channel)
                    {
                        output.append(&mut tui.leave_screen()?);
                        finish_tui = Some((audit, "ok", None));
                    }
                    close = true;
                }
                Some(TuiAction::ProbeAll) => {
                    if let Some(ChannelState::Tui { tui, .. }) = channels.get_mut(&channel) {
                        tui.app.status_notice = Some("Probing authorized assets...".to_string());
                        output.append(&mut tui.render()?);
                        probe_tui = true;
                    }
                }
                Some(TuiAction::Connect(asset)) => {
                    if asset.credential_id.is_none() {
                        output.extend_from_slice(
                            b"\r\nAsset has no managed credential; use ProxyJump instead.\r\n",
                        );
                    } else {
                        let target_guard = self.auth_info().map(|auth| {
                            self.active_sessions.select_tui_target(
                                &auth.key_id,
                                self.peer_ip,
                                (*asset).clone(),
                            )
                        });
                        if let Some(ChannelState::Tui { mut tui, audit, .. }) =
                            channels.remove(&channel)
                        {
                            *self
                                .selected_asset
                                .lock()
                                .expect("selected_asset mutex poisoned") = Some((*asset).clone());
                            output.append(&mut tui.leave_screen()?);
                            finish_tui = Some((audit, "connected", None));
                            connect = Some((*asset, *tui, target_guard));
                        }
                    }
                }
            }
        }
        send_tui_output(session, channel, output)?;
        if probe_tui {
            if let Some(auth) = self.auth_info() {
                let db = self.db.clone();
                let master_key = Arc::clone(&self.master_key);
                let channels = self.channels.clone();
                let handle = session.handle();
                let timeout = self.config.connect_timeout();
                tokio::spawn(async move {
                    let Ok(assets) = db.list_assets_for_key(&auth.key_id).await else {
                        return;
                    };
                    let count = assets.len();
                    let _ =
                        crate::probe::probe_assets(&db, Some(master_key), &assets, timeout).await;
                    let Ok(fresh) = db.list_assets_for_key(&auth.key_id).await else {
                        return;
                    };
                    let rendered = {
                        let mut guard = channels.lock().await;
                        if let Some(ChannelState::Tui { tui, .. }) = guard.get_mut(&channel) {
                            tui.set_assets(fresh);
                            tui.app.status_notice = Some(format!("Probed {count} asset(s)"));
                            tui.render().ok()
                        } else {
                            None
                        }
                    };
                    if let Some(bytes) = rendered {
                        if !bytes.is_empty() {
                            let _ = handle.data(channel, bytes).await;
                        }
                    }
                });
            }
        }
        if let Some((audit, status, error)) = finish_tui {
            let session_id = audit.id().to_string();
            let _ = audit.finish(&self.db, status, error.as_deref()).await;
            self.active_sessions.unregister(&session_id).await;
        }
        if close {
            session.exit_status_request(channel, 0)?;
            session.eof(channel)?;
            session.close(channel)?;
        }
        if let Some((asset, tui, target_guard)) = connect {
            if let Some(cached) = self.direct_client.lock().await.take() {
                cached.disconnect("switching tui target").await;
            }
            self.start_managed(
                channel,
                session.handle(),
                asset,
                Some(tui),
                target_guard,
                ManagedSessionMode::Tui,
                None,
            )
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
                    let _ = control.input.send(BridgeInput::Eof);
                }
                Some(ChannelState::Tui { .. }) => {
                    if let Some(ChannelState::Tui { mut tui, audit, .. }) =
                        channels.remove(&channel)
                    {
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
            let session_id = audit.id().to_string();
            let _ = audit.finish(&self.db, "ok", None).await;
            self.active_sessions.unregister(&session_id).await;
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let state = self.channels.lock().await.remove(&channel);
        if let Some(ChannelState::Tui { mut tui, audit, .. }) = state {
            if let Ok(output) = tui.leave_screen() {
                let _ = send_tui_output(session, channel, output);
            }
            let session_id = audit.id().to_string();
            let _ = audit.finish(&self.db, "ok", None).await;
            self.active_sessions.unregister(&session_id).await;
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

pub(crate) fn accepts_sftp_request(name: &str, asset: Option<&Asset>) -> bool {
    name == "sftp" && accepts_managed_ssh_asset(asset)
}

fn accepts_managed_ssh_asset(asset: Option<&Asset>) -> bool {
    asset.is_some_and(|asset| {
        asset.protocol == hop_core::ASSET_PROTOCOL_SSH && asset.credential_id.is_some()
    })
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

fn log_ssh_session_error(client_ip: SocketAddr, error: &anyhow::Error) {
    if is_client_disconnect(error) {
        warn!(%client_ip, %error, "ssh client disconnected");
    } else {
        error!(%client_ip, %error, "ssh session error");
    }
}

fn is_client_disconnect(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(is_disconnect_io_error)
            || cause
                .downcast_ref::<russh::Error>()
                .is_some_and(|error| match error {
                    russh::Error::IO(error) => is_disconnect_io_error(error),
                    russh::Error::Disconnect | russh::Error::HUP => true,
                    _ => false,
                })
    })
}

fn is_disconnect_io_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe | io::ErrorKind::UnexpectedEof
    ) || matches!(error.raw_os_error(), Some(10054) | Some(104))
}

#[cfg(test)]
mod tests {
    use hop_core::{
        encrypt_envelope, AssetAccessMode, AuthType, NewAsset, NewAuthorizedKey, NewCredential,
        NewSession,
    };
    use russh::{
        client::Msg as ClientMsg,
        keys::{key::safe_rng, Algorithm, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
        server::{Msg as ServerMsg, Server as _},
        ChannelMsg,
    };
    use std::sync::Mutex as StdMutex;
    use tokio::io::AsyncReadExt;

    use super::*;

    fn test_handler(db: Catalog) -> HopSshHandler {
        HopSshHandler {
            db,
            config: HopConfig::default(),
            master_key: Arc::new(MasterKey::from_bytes([0; 32])),
            active_sessions: ActiveSessionRegistry::default(),
            auth: None,
            direct_asset: None,
            selected_asset: Arc::new(StdMutex::new(None)),
            direct_client: Arc::new(Mutex::new(None)),
            peer_ip: None,
            client_ip: None,
            channels: Arc::new(Mutex::new(HashMap::new())),
            ptys: HashMap::new(),
            pty_terms: HashMap::new(),
        }
    }

    #[derive(Clone)]
    struct ExecTargetServer {
        expected_user: String,
        expected_password: String,
        auth_count: Arc<std::sync::atomic::AtomicUsize>,
        commands: Arc<StdMutex<Vec<Vec<u8>>>>,
        ptys: Arc<StdMutex<Vec<(u32, u32)>>>,
        forwards: Arc<StdMutex<Vec<(String, u32)>>>,
        subsystems: Arc<StdMutex<Vec<String>>>,
        session_channels: std::collections::HashSet<ChannelId>,
    }

    impl Default for ExecTargetServer {
        fn default() -> Self {
            Self::with_credentials("target-user", "target-password")
        }
    }

    impl ExecTargetServer {
        fn with_credentials(user: &str, password: &str) -> Self {
            Self {
                expected_user: user.to_string(),
                expected_password: password.to_string(),
                auth_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                commands: Arc::new(StdMutex::new(Vec::new())),
                ptys: Arc::new(StdMutex::new(Vec::new())),
                forwards: Arc::new(StdMutex::new(Vec::new())),
                subsystems: Arc::new(StdMutex::new(Vec::new())),
                session_channels: std::collections::HashSet::new(),
            }
        }
    }

    impl server::Server for ExecTargetServer {
        type Handler = Self;

        fn new_client(&mut self, _peer_addr: Option<SocketAddr>) -> Self::Handler {
            self.clone()
        }
    }

    impl server::Handler for ExecTargetServer {
        type Error = russh::Error;

        async fn auth_password(
            &mut self,
            user: &str,
            password: &str,
        ) -> Result<server::Auth, Self::Error> {
            if user == self.expected_user && password == self.expected_password {
                self.auth_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(server::Auth::Accept)
            } else {
                Ok(server::Auth::reject())
            }
        }

        async fn channel_open_session(
            &mut self,
            channel: Channel<ServerMsg>,
            reply: server::ChannelOpenHandle,
            _session: &mut Session,
        ) -> Result<(), Self::Error> {
            self.session_channels.insert(channel.id());
            reply.accept().await;
            Ok(())
        }

        async fn channel_open_direct_tcpip(
            &mut self,
            channel: Channel<ServerMsg>,
            host_to_connect: &str,
            port_to_connect: u32,
            _originator_address: &str,
            _originator_port: u32,
            reply: server::ChannelOpenHandle,
            _session: &mut Session,
        ) -> Result<(), Self::Error> {
            self.forwards
                .lock()
                .unwrap()
                .push((host_to_connect.to_string(), port_to_connect));
            let Ok(port) = u16::try_from(port_to_connect) else {
                reply.reject(ChannelOpenFailure::ConnectFailed).await;
                return Ok(());
            };
            match tokio::net::TcpStream::connect((host_to_connect, port)).await {
                Ok(mut tcp) => {
                    reply.accept().await;
                    tokio::spawn(async move {
                        let mut stream = channel.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
                    });
                }
                Err(_) => {
                    reply.reject(ChannelOpenFailure::ConnectFailed).await;
                }
            }
            Ok(())
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
            self.ptys.lock().unwrap().push((col_width, row_height));
            session.channel_success(channel)?;
            Ok(())
        }

        async fn shell_request(
            &mut self,
            channel: ChannelId,
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            session.channel_success(channel)?;
            session.data(channel, b"WELCOME-SHELL\n".to_vec())?;
            Ok(())
        }

        async fn subsystem_request(
            &mut self,
            channel: ChannelId,
            name: &str,
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            self.subsystems.lock().unwrap().push(name.to_string());
            if name == "sftp" {
                session.channel_success(channel)?;
            } else {
                session.channel_failure(channel)?;
            }
            Ok(())
        }

        async fn exec_request(
            &mut self,
            channel: ChannelId,
            data: &[u8],
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            self.commands.lock().unwrap().push(data.to_vec());
            session.channel_success(channel)?;
            Ok(())
        }

        async fn data(
            &mut self,
            channel: ChannelId,
            data: &[u8],
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            if self.session_channels.contains(&channel) {
                session.data(channel, data.to_vec())?;
            }
            Ok(())
        }

        async fn channel_eof(
            &mut self,
            channel: ChannelId,
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            if self.session_channels.remove(&channel) {
                session.extended_data(channel, 1, b"target stderr\n".to_vec())?;
                session.exit_status_request(channel, 42)?;
                session.data(channel, b"after status\n".to_vec())?;
                session.eof(channel)?;
                session.close(channel)?;
            }
            Ok(())
        }
    }

    #[derive(Clone)]
    struct AcceptAnyServerKey;

    impl russh::client::Handler for AcceptAnyServerKey {
        type Error = russh::Error;

        async fn check_server_key(
            &mut self,
            _server_public_key: &PublicKeyOrCertificate,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }
    }

    async fn wait_for_channel_success(channel: &mut Channel<ClientMsg>) {
        let message = tokio::time::timeout(Duration::from_secs(5), channel.wait())
            .await
            .expect("SSH channel request timed out");
        assert!(matches!(message, Some(ChannelMsg::Success)));
    }

    async fn run_exec_channel(
        session: &russh::client::Handle<AcceptAnyServerKey>,
        command: &[u8],
        pty: Option<(u32, u32)>,
        stdin: &[u8],
    ) -> (Vec<u8>, Vec<u8>, Option<u32>) {
        let mut channel = session.channel_open_session().await.unwrap();
        if let Some((width, height)) = pty {
            channel
                .request_pty(true, "xterm-256color", width, height, 0, 0, &[])
                .await
                .unwrap();
            wait_for_channel_success(&mut channel).await;
        }
        channel.exec(true, command.to_vec()).await.unwrap();
        wait_for_channel_success(&mut channel).await;
        if !stdin.is_empty() {
            channel.data(stdin).await.unwrap();
        }
        channel.eof().await.unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status = None;
        loop {
            let message = tokio::time::timeout(Duration::from_secs(5), channel.wait())
                .await
                .expect("SSH exec response timed out");
            match message {
                Some(ChannelMsg::Data { data }) => stdout.extend_from_slice(&data),
                Some(ChannelMsg::ExtendedData { data, ext: 1 }) => stderr.extend_from_slice(&data),
                Some(ChannelMsg::ExitStatus {
                    exit_status: status,
                }) => exit_status = Some(status),
                Some(ChannelMsg::Eof | ChannelMsg::Close) | None => break,
                _ => {}
            }
        }
        (stdout, stderr, exit_status)
    }

    #[tokio::test]
    async fn managed_exec_forwards_streams_exit_status_and_optional_pty() {
        let mut target = ExecTargetServer::default();
        let target_state = target.clone();
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target_listener.local_addr().unwrap();
        let target_config = Arc::new(server::Config {
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![
                russh::keys::PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap(),
            ],
            ..Default::default()
        });
        let target_task =
            tokio::spawn(
                async move { target.run_on_socket(target_config, &target_listener).await },
            );

        let db = Catalog::in_memory().await.unwrap();
        let master_key = Arc::new(MasterKey::from_bytes([7; 32]));
        let credential_id = "exec-target-credential";
        let credential = db
            .add_credential(NewCredential {
                id: Some(credential_id.to_string()),
                name: "exec target".to_string(),
                username: "target-user".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some(
                    encrypt_envelope(
                        &master_key,
                        &format!("{credential_id}:password"),
                        b"target-password",
                    )
                    .unwrap(),
                ),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();
        let mut asset = NewAsset::new(
            "exec-target",
            target_addr.ip().to_string(),
            i64::from(target_addr.port()),
        );
        asset.credential_id = Some(credential.id);
        db.add_asset(asset).await.unwrap();

        let client_key =
            russh::keys::PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
        let client_public_key = client_key.public_key();
        db.add_authorized_key(NewAuthorizedKey::new(
            "exec client",
            client_public_key.to_openssh().unwrap(),
            key_fingerprint(client_public_key),
        ))
        .await
        .unwrap();

        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let hop_addr = probe.local_addr().unwrap();
        drop(probe);
        let temp = tempfile::tempdir().unwrap();
        let config = HopConfig {
            listen: hop_addr.to_string(),
            data_dir: temp.path().to_path_buf(),
            ssh: hop_core::config::SshConfig {
                banner: String::new(),
                keepalive_interval: 0,
                connect_timeout: 5,
                ..hop_core::config::SshConfig::default()
            },
            ..HopConfig::default()
        };
        let (_shutdown_tx, shutdown) = watch::channel(false);
        let hop_task = tokio::spawn(serve_ssh_with_shutdown(
            hop_addr,
            config,
            db.clone(),
            master_key,
            ActiveSessionRegistry::default(),
            shutdown,
        ));

        let mut hop_client = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match russh::client::connect(
                    Arc::new(russh::client::Config::default()),
                    hop_addr,
                    AcceptAnyServerKey,
                )
                .await
                {
                    Ok(client) => break Some(client),
                    Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .expect("Hop SSH server did not start")
        .expect("Hop SSH client was not created");
        let hash_alg = hop_client
            .best_supported_rsa_hash()
            .await
            .unwrap()
            .flatten();
        let auth = hop_client
            .authenticate_publickey(
                "exec-target",
                PrivateKeyWithHashAlg::new(Arc::new(client_key), hash_alg),
            )
            .await
            .unwrap();
        assert!(auth.success());

        let (stdout, stderr, exit_status) =
            run_exec_channel(&hop_client, b"cat", None, b"stdin payload\n").await;
        assert_eq!(stdout, b"stdin payload\nafter status\n");
        assert_eq!(stderr, b"target stderr\n");
        assert_eq!(exit_status, Some(42));
        assert!(target_state.ptys.lock().unwrap().is_empty());

        let (_, _, exit_status) =
            run_exec_channel(&hop_client, b"tty-check", Some((123, 45)), b"").await;
        assert_eq!(exit_status, Some(42));
        assert_eq!(&*target_state.ptys.lock().unwrap(), &[(123, 45)]);
        assert_eq!(
            &*target_state.commands.lock().unwrap(),
            &[b"cat".to_vec(), b"tty-check".to_vec()]
        );

        let sessions = db.list_sessions(10).await.unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions
            .iter()
            .all(|session| session.mode == "exec" && session.status == "ok"));

        hop_client
            .disconnect(russh::Disconnect::ByApplication, "test complete", "en")
            .await
            .ok();
        hop_task.abort();
        target_task.abort();
    }

    #[tokio::test]
    async fn ssh_listener_stops_accepting_after_shutdown_notification() {
        let temp = tempfile::tempdir().unwrap();
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = probe.local_addr().unwrap();
        drop(probe);
        let config = HopConfig {
            listen: address.to_string(),
            data_dir: temp.path().to_path_buf(),
            ..HopConfig::default()
        };
        let (shutdown_tx, shutdown) = watch::channel(false);
        let task = tokio::spawn(serve_ssh_with_shutdown(
            address,
            config,
            Catalog::in_memory().await.unwrap(),
            Arc::new(MasterKey::from_bytes([9; 32])),
            ActiveSessionRegistry::default(),
            shutdown,
        ));

        let mut connected = false;
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(address).await.is_ok() {
                connected = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(connected, "SSH server did not bind its listener");

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("SSH server did not shut down")
            .unwrap()
            .unwrap();
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
    }

    #[tokio::test]
    async fn ssh_listener_drops_disallowed_clients_before_handshake() {
        let temp = tempfile::tempdir().unwrap();
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = probe.local_addr().unwrap();
        drop(probe);
        let config = HopConfig {
            listen: address.to_string(),
            data_dir: temp.path().to_path_buf(),
            ssh: hop_core::config::SshConfig {
                allowed_client_ip_ranges: Vec::new(),
                ..hop_core::config::SshConfig::default()
            },
            ..HopConfig::default()
        };
        let (shutdown_tx, shutdown) = watch::channel(false);
        let db = Catalog::in_memory().await.unwrap();
        let task = tokio::spawn(serve_ssh_with_shutdown(
            address,
            config,
            db.clone(),
            Arc::new(MasterKey::from_bytes([8; 32])),
            ActiveSessionRegistry::default(),
            shutdown,
        ));

        let mut client = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match tokio::net::TcpStream::connect(address).await {
                    Ok(client) => break client,
                    Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
        })
        .await
        .expect("SSH server did not bind its listener");
        let mut buffer = [0; 1];
        let read = tokio::time::timeout(Duration::from_secs(2), client.read(&mut buffer))
            .await
            .expect("disallowed SSH client was not closed")
            .expect("reading disallowed SSH client failed");
        assert_eq!(read, 0, "disallowed client received an SSH handshake");
        assert!(db.list_sessions(10).await.unwrap().is_empty());

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("SSH server did not shut down")
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn active_tui_session_finish_marks_record_done() {
        let db = Catalog::in_memory().await.unwrap();
        let session = db
            .start_session(NewSession {
                key_id: None,
                key_finger: "SHA256:test".to_string(),
                key_name: Some("tester".to_string()),
                mode: "tui".to_string(),
                asset_id: None,
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
    fn ingress_advertises_only_public_key_authentication() {
        assert_eq!(&*ingress_auth_methods(), &[MethodKind::PublicKey]);
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

    #[test]
    fn wrapped_early_eof_is_treated_as_client_disconnect() {
        let error = anyhow::Error::new(russh::Error::IO(std::io::Error::from(
            std::io::ErrorKind::UnexpectedEof,
        )));

        assert!(is_client_disconnect(&error));
    }

    #[test]
    fn protocol_disconnect_is_treated_as_client_disconnect() {
        assert!(is_client_disconnect(&anyhow::Error::new(
            russh::Error::Disconnect
        )));
        assert!(!is_client_disconnect(&anyhow::anyhow!(
            "catalog query failed"
        )));
    }

    #[test]
    fn direct_username_uses_asset_name_as_target_and_reserves_menu() {
        let request = parse_direct_username("web-prod-01").unwrap().unwrap();

        assert_eq!(request.target, "web-prod-01");
        assert!(parse_direct_username("menu").unwrap().is_none());
        assert!(parse_direct_username("+tui").unwrap().is_none());
        assert!(parse_direct_username("alice@web-prod-01").is_err());
        assert!(parse_direct_username("").is_err());
    }

    #[test]
    fn sftp_is_only_accepted_for_managed_ssh_assets() {
        let mut asset = Asset {
            id: "asset-1".to_string(),
            name: "web-prod-01".to_string(),
            protocol: hop_core::ASSET_PROTOCOL_SSH.to_string(),
            hostname: "10.0.0.10".to_string(),
            port: 22,
            display_name: None,
            description: None,
            tags: Vec::new(),
            credential_id: Some("cred-1".to_string()),
            via_asset_id: None,
            allow_tcp_forwarding: true,
            created_at: None,
            updated_at: None,
            health: hop_core::AssetHealth::unknown("asset-1"),
        };

        assert!(accepts_sftp_request("sftp", Some(&asset)));
        assert!(!accepts_sftp_request("shell", Some(&asset)));
        assert!(!accepts_sftp_request("sftp", None));

        asset.credential_id = None;
        assert!(!accepts_sftp_request("sftp", Some(&asset)));
        asset.protocol = hop_core::ASSET_PROTOCOL_TCP.to_string();
        asset.credential_id = Some("cred-1".to_string());
        assert!(!accepts_sftp_request("sftp", Some(&asset)));
    }

    #[tokio::test]
    async fn proxy_only_asset_username_falls_back_cleanly_for_proxyjump() {
        let db = Catalog::in_memory().await.unwrap();
        let key = db
            .add_authorized_key(NewAuthorizedKey::new(
                "laptop",
                "ssh-ed25519 AAAA-test",
                "SHA256:test",
            ))
            .await
            .unwrap();
        db.add_asset(NewAsset::new("proxy-box", "10.0.0.99", 22))
            .await
            .unwrap();
        let handler = test_handler(db);

        assert!(handler
            .resolve_direct_asset_for_key("proxy-box", &key.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn direct_asset_resolution_uses_stable_authenticated_key_id() {
        let db = Catalog::in_memory().await.unwrap();
        let key = db
            .add_authorized_key(NewAuthorizedKey::new(
                "laptop",
                "ssh-ed25519 AAAA-test",
                "SHA256:test",
            ))
            .await
            .unwrap();
        let credential = db
            .add_credential(NewCredential {
                id: Some("cred-1".to_string()),
                name: "deploy".to_string(),
                username: "deploy".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some("encrypted-password".to_string()),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();
        let mut asset = NewAsset::new("web-prod-01", "10.0.0.10", 22);
        asset.credential_id = Some(credential.id);
        db.add_asset(asset).await.unwrap();
        let handler = test_handler(db);

        let asset = handler
            .resolve_direct_asset_for_key("web-prod-01", &key.id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(asset.name, "web-prod-01");
    }

    #[tokio::test]
    async fn direct_asset_resolution_rejects_unassigned_asset() {
        let db = Catalog::in_memory().await.unwrap();
        let key = db
            .add_authorized_key(NewAuthorizedKey::new(
                "laptop",
                "ssh-ed25519 AAAA-test",
                "SHA256:test",
            ))
            .await
            .unwrap();
        let credential = db
            .add_credential(NewCredential {
                id: Some("cred-1".to_string()),
                name: "deploy".to_string(),
                username: "deploy".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some("encrypted-password".to_string()),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();
        let mut asset = NewAsset::new("web-prod-01", "10.0.0.10", 22);
        asset.credential_id = Some(credential.id);
        db.add_asset(asset).await.unwrap();
        db.set_authorized_key_access(&key.id, AssetAccessMode::Restricted, &[])
            .await
            .unwrap();
        let handler = test_handler(db);

        assert!(handler
            .resolve_direct_asset_for_key("web-prod-01", &key.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn tui_direct_exec_and_sftp_share_the_same_managed_authorization_gate() {
        let db = Catalog::in_memory().await.unwrap();
        let key = db
            .add_authorized_key_with_access(
                NewAuthorizedKey::new("empty", "ssh-ed25519 AAAA-empty", "SHA256:empty"),
                AssetAccessMode::Restricted,
                &[],
            )
            .await
            .unwrap();
        let asset = db
            .add_asset(NewAsset::new("server", "192.0.2.10", 22))
            .await
            .unwrap();
        for mode in [
            ManagedSessionMode::Tui,
            ManagedSessionMode::Direct,
            ManagedSessionMode::Exec,
            ManagedSessionMode::Sftp,
        ] {
            assert!(!managed_mode_is_authorized(&db, &key.id, &asset.id, mode)
                .await
                .unwrap());
        }
        db.set_authorized_key_access(
            &key.id,
            AssetAccessMode::Restricted,
            std::slice::from_ref(&asset.id),
        )
        .await
        .unwrap();
        for mode in [
            ManagedSessionMode::Tui,
            ManagedSessionMode::Direct,
            ManagedSessionMode::Exec,
            ManagedSessionMode::Sftp,
        ] {
            assert!(managed_mode_is_authorized(&db, &key.id, &asset.id, mode)
                .await
                .unwrap());
        }
    }

    #[tokio::test]
    async fn chained_via_asset_exec_and_probe_traverse_jump_host_and_record_tofu_and_samples() {
        let mut jump_server = ExecTargetServer::with_credentials("jump-user", "jump-password");
        let jump_state = jump_server.clone();
        let jump_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let jump_addr = jump_listener.local_addr().unwrap();
        let jump_config = Arc::new(server::Config {
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![
                russh::keys::PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap(),
            ],
            ..Default::default()
        });
        let jump_task =
            tokio::spawn(
                async move { jump_server.run_on_socket(jump_config, &jump_listener).await },
            );

        let mut leaf_server = ExecTargetServer::with_credentials("leaf-user", "leaf-password");
        let leaf_state = leaf_server.clone();
        let leaf_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let leaf_addr = leaf_listener.local_addr().unwrap();
        let leaf_config = Arc::new(server::Config {
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![
                russh::keys::PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap(),
            ],
            ..Default::default()
        });
        let leaf_task =
            tokio::spawn(
                async move { leaf_server.run_on_socket(leaf_config, &leaf_listener).await },
            );

        let db = Catalog::in_memory().await.unwrap();
        let master_key = Arc::new(MasterKey::from_bytes([11; 32]));

        let jump_cred_id = "jump-cred";
        let jump_cred = db
            .add_credential(NewCredential {
                id: Some(jump_cred_id.to_string()),
                name: "jump cred".to_string(),
                username: "jump-user".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some(
                    encrypt_envelope(
                        &master_key,
                        &format!("{jump_cred_id}:password"),
                        b"jump-password",
                    )
                    .unwrap(),
                ),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();
        let leaf_cred_id = "leaf-cred";
        let leaf_cred = db
            .add_credential(NewCredential {
                id: Some(leaf_cred_id.to_string()),
                name: "leaf cred".to_string(),
                username: "leaf-user".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some(
                    encrypt_envelope(
                        &master_key,
                        &format!("{leaf_cred_id}:password"),
                        b"leaf-password",
                    )
                    .unwrap(),
                ),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();

        let mut jump_new = NewAsset::new(
            "hk-relay",
            jump_addr.ip().to_string(),
            i64::from(jump_addr.port()),
        );
        jump_new.credential_id = Some(jump_cred.id);
        let jump_asset = db.add_asset(jump_new).await.unwrap();

        let mut leaf_new = NewAsset::new(
            "jp-private",
            leaf_addr.ip().to_string(),
            i64::from(leaf_addr.port()),
        );
        leaf_new.credential_id = Some(leaf_cred.id);
        leaf_new.via_asset_id = Some(jump_asset.id.clone());
        let leaf_asset = db.add_asset(leaf_new).await.unwrap();

        let probed =
            crate::probe::probe_asset(&db, Some(&master_key), &leaf_asset, Duration::from_secs(5))
                .await
                .unwrap();
        assert_eq!(probed.health.status, "healthy");
        assert!(probed.health.latency_ms.is_some());
        let samples = db
            .list_asset_probe_samples(&leaf_asset.id, 10)
            .await
            .unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].status, "healthy");

        let client_key =
            russh::keys::PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
        let client_public_key = client_key.public_key();
        db.add_authorized_key(NewAuthorizedKey::new(
            "dev laptop",
            client_public_key.to_openssh().unwrap(),
            key_fingerprint(client_public_key),
        ))
        .await
        .unwrap();

        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let hop_addr = probe.local_addr().unwrap();
        drop(probe);
        let temp = tempfile::tempdir().unwrap();
        let config = HopConfig {
            listen: hop_addr.to_string(),
            data_dir: temp.path().to_path_buf(),
            ssh: hop_core::config::SshConfig {
                banner: String::new(),
                keepalive_interval: 0,
                connect_timeout: 5,
                ..hop_core::config::SshConfig::default()
            },
            ..HopConfig::default()
        };
        let (_shutdown_tx, shutdown) = watch::channel(false);
        let hop_task = tokio::spawn(serve_ssh_with_shutdown(
            hop_addr,
            config,
            db.clone(),
            master_key,
            ActiveSessionRegistry::default(),
            shutdown,
        ));

        let mut hop_client = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match russh::client::connect(
                    Arc::new(russh::client::Config::default()),
                    hop_addr,
                    AcceptAnyServerKey,
                )
                .await
                {
                    Ok(client) => break Some(client),
                    Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .expect("Hop SSH server did not start")
        .expect("Hop SSH client was not created");

        let hash_alg = hop_client
            .best_supported_rsa_hash()
            .await
            .unwrap()
            .flatten();
        let auth = hop_client
            .authenticate_publickey(
                "jp-private",
                PrivateKeyWithHashAlg::new(Arc::new(client_key), hash_alg),
            )
            .await
            .unwrap();
        assert!(auth.success());

        let (stdout, stderr, exit_status) =
            run_exec_channel(&hop_client, b"cat", None, b"chained payload\n").await;
        assert_eq!(
            stdout,
            b"chained payload\nafter status\n",
            "unexpected stdout; stderr={} exit={:?}",
            String::from_utf8_lossy(&stderr),
            exit_status
        );
        assert_eq!(stderr, b"target stderr\n");
        assert_eq!(exit_status, Some(42));
        assert_eq!(&*leaf_state.commands.lock().unwrap(), &[b"cat".to_vec()]);
        assert_eq!(
            jump_state.forwards.lock().unwrap().len(),
            2,
            "jump host should have forwarded both probe and exec streams"
        );
        assert_eq!(db.list_known_hosts().await.unwrap().len(), 2);

        hop_client
            .disconnect(russh::Disconnect::ByApplication, "done", "en")
            .await
            .ok();
        hop_task.abort();
        jump_task.abort();
        leaf_task.abort();
    }

    #[tokio::test]
    async fn managed_direct_tcpip_multiplexes_multiple_port_forwards_over_cached_target_session() {
        use tokio::io::AsyncWriteExt;

        let echo_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let echo_addr = echo_listener.local_addr().unwrap();
        let echo_task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = echo_listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 256];
                    if let Ok(n) = stream.read(&mut buf).await {
                        let mut reply = b"echo:".to_vec();
                        reply.extend_from_slice(&buf[..n]);
                        let _ = stream.write_all(&reply).await;
                        let _ = stream.shutdown().await;
                    }
                });
            }
        });

        let mut target = ExecTargetServer::default();
        let target_state = target.clone();
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target_listener.local_addr().unwrap();
        let target_config = Arc::new(server::Config {
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![
                russh::keys::PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap(),
            ],
            ..Default::default()
        });
        let target_task =
            tokio::spawn(
                async move { target.run_on_socket(target_config, &target_listener).await },
            );

        let db = Catalog::in_memory().await.unwrap();
        let master_key = Arc::new(MasterKey::from_bytes([12; 32]));
        let credential_id = "fwd-target-cred";
        let credential = db
            .add_credential(NewCredential {
                id: Some(credential_id.to_string()),
                name: "fwd target".to_string(),
                username: "target-user".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some(
                    encrypt_envelope(
                        &master_key,
                        &format!("{credential_id}:password"),
                        b"target-password",
                    )
                    .unwrap(),
                ),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();

        let mut asset = NewAsset::new(
            "cursor-dev",
            target_addr.ip().to_string(),
            i64::from(target_addr.port()),
        );
        asset.credential_id = Some(credential.id);
        db.add_asset(asset).await.unwrap();

        let client_key =
            russh::keys::PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
        let client_public_key = client_key.public_key();
        db.add_authorized_key(NewAuthorizedKey::new(
            "cursor ide",
            client_public_key.to_openssh().unwrap(),
            key_fingerprint(client_public_key),
        ))
        .await
        .unwrap();

        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let hop_addr = probe.local_addr().unwrap();
        drop(probe);
        let temp = tempfile::tempdir().unwrap();
        let config = HopConfig {
            listen: hop_addr.to_string(),
            data_dir: temp.path().to_path_buf(),
            ssh: hop_core::config::SshConfig {
                banner: String::new(),
                keepalive_interval: 0,
                connect_timeout: 5,
                ..hop_core::config::SshConfig::default()
            },
            ..HopConfig::default()
        };
        let (_shutdown_tx, shutdown) = watch::channel(false);
        let hop_task = tokio::spawn(serve_ssh_with_shutdown(
            hop_addr,
            config,
            db.clone(),
            master_key,
            ActiveSessionRegistry::default(),
            shutdown,
        ));

        let mut hop_client = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match russh::client::connect(
                    Arc::new(russh::client::Config::default()),
                    hop_addr,
                    AcceptAnyServerKey,
                )
                .await
                {
                    Ok(client) => break Some(client),
                    Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .unwrap()
        .unwrap();
        let hash_alg = hop_client
            .best_supported_rsa_hash()
            .await
            .unwrap()
            .flatten();
        let auth = hop_client
            .authenticate_publickey(
                "cursor-dev",
                PrivateKeyWithHashAlg::new(Arc::new(client_key), hash_alg),
            )
            .await
            .unwrap();
        assert!(auth.success());

        let (ch1, ch2) = tokio::join!(
            hop_client.channel_open_direct_tcpip(
                echo_addr.ip().to_string(),
                u32::from(echo_addr.port()),
                "127.0.0.1",
                51001,
            ),
            hop_client.channel_open_direct_tcpip(
                echo_addr.ip().to_string(),
                u32::from(echo_addr.port()),
                "127.0.0.1",
                51002,
            )
        );
        let mut s1 = ch1.unwrap().into_stream();
        let mut s2 = ch2.unwrap().into_stream();
        s1.write_all(b"cursor-port-1").await.unwrap();
        s2.write_all(b"socks-port-2").await.unwrap();
        let mut out1 = Vec::new();
        let mut out2 = Vec::new();
        s1.read_to_end(&mut out1).await.unwrap();
        s2.read_to_end(&mut out2).await.unwrap();
        assert_eq!(out1, b"echo:cursor-port-1");
        assert_eq!(out2, b"echo:socks-port-2");
        drop(s1);
        drop(s2);

        assert_eq!(
            target_state
                .auth_count
                .load(std::sync::atomic::Ordering::SeqCst),
            1,
            "multiple direct-tcpip channels must multiplex over a single cached target SSH session"
        );

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let sessions = db.list_sessions(10).await.unwrap();
                if sessions.len() == 2 && sessions.iter().all(|s| s.ended_at.is_some()) {
                    assert!(sessions
                        .iter()
                        .all(|s| s.mode == "managed-tcp-forward" && s.status == "ok"));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("managed-tcp-forward sessions did not complete cleanly");

        hop_client
            .disconnect(russh::Disconnect::ByApplication, "done", "en")
            .await
            .ok();
        hop_task.abort();
        target_task.abort();
        echo_task.abort();
    }

    #[tokio::test]
    async fn managed_direct_tcpip_honors_per_asset_forwarding_disable_policy() {
        let mut target = ExecTargetServer::default();
        let target_state = target.clone();
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target_listener.local_addr().unwrap();
        let target_config = Arc::new(server::Config {
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![
                russh::keys::PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap(),
            ],
            ..Default::default()
        });
        let target_task =
            tokio::spawn(
                async move { target.run_on_socket(target_config, &target_listener).await },
            );

        let db = Catalog::in_memory().await.unwrap();
        let master_key = Arc::new(MasterKey::from_bytes([13; 32]));
        let credential_id = "locked-cred";
        let credential = db
            .add_credential(NewCredential {
                id: Some(credential_id.to_string()),
                name: "locked target".to_string(),
                username: "target-user".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some(
                    encrypt_envelope(
                        &master_key,
                        &format!("{credential_id}:password"),
                        b"target-password",
                    )
                    .unwrap(),
                ),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();

        let mut asset = NewAsset::new(
            "locked-box",
            target_addr.ip().to_string(),
            i64::from(target_addr.port()),
        );
        asset.credential_id = Some(credential.id);
        asset.allow_tcp_forwarding = false;
        db.add_asset(asset).await.unwrap();

        let client_key =
            russh::keys::PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
        let client_public_key = client_key.public_key();
        db.add_authorized_key(NewAuthorizedKey::new(
            "restricted laptop",
            client_public_key.to_openssh().unwrap(),
            key_fingerprint(client_public_key),
        ))
        .await
        .unwrap();

        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let hop_addr = probe.local_addr().unwrap();
        drop(probe);
        let temp = tempfile::tempdir().unwrap();
        let config = HopConfig {
            listen: hop_addr.to_string(),
            data_dir: temp.path().to_path_buf(),
            ssh: hop_core::config::SshConfig {
                banner: String::new(),
                keepalive_interval: 0,
                connect_timeout: 5,
                ..hop_core::config::SshConfig::default()
            },
            ..HopConfig::default()
        };
        let (_shutdown_tx, shutdown) = watch::channel(false);
        let hop_task = tokio::spawn(serve_ssh_with_shutdown(
            hop_addr,
            config,
            db.clone(),
            master_key,
            ActiveSessionRegistry::default(),
            shutdown,
        ));

        let mut hop_client = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match russh::client::connect(
                    Arc::new(russh::client::Config::default()),
                    hop_addr,
                    AcceptAnyServerKey,
                )
                .await
                {
                    Ok(client) => break Some(client),
                    Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .unwrap()
        .unwrap();
        let hash_alg = hop_client
            .best_supported_rsa_hash()
            .await
            .unwrap()
            .flatten();
        let auth = hop_client
            .authenticate_publickey(
                "locked-box",
                PrivateKeyWithHashAlg::new(Arc::new(client_key), hash_alg),
            )
            .await
            .unwrap();
        assert!(auth.success());

        let result = hop_client
            .channel_open_direct_tcpip("127.0.0.1", 8080, "127.0.0.1", 52000)
            .await;
        assert!(
            result.is_err(),
            "disabled allow_tcp_forwarding must reject direct-tcpip"
        );
        assert_eq!(
            target_state
                .auth_count
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );

        let sessions = db.list_sessions(10).await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].mode, "managed-tcp-forward");
        assert_eq!(sessions[0].status, "failed");
        assert_eq!(
            sessions[0].error.as_deref(),
            Some("tcp forwarding disabled for asset")
        );

        let audits = db.list_audit_events(10).await.unwrap();
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].action, "session.forward_denied");
        assert_eq!(audits[0].target_type, "session");
        assert_eq!(audits[0].result, "denied");
        assert_eq!(audits[0].actor_label, "ssh-ingress");
        assert!(audits[0]
            .details_json
            .as_deref()
            .unwrap_or_default()
            .contains("tcp forwarding disabled for asset"));

        hop_client
            .disconnect(russh::Disconnect::ByApplication, "done", "en")
            .await
            .ok();
        hop_task.abort();
        target_task.abort();
    }

    #[tokio::test]
    async fn proxy_direct_tcpip_distinguishes_unauthorized_vs_missing_targets_in_audit() {
        let db = Catalog::in_memory().await.unwrap();
        let master_key = Arc::new(MasterKey::from_bytes([17; 32]));
        let protected = db
            .add_asset(NewAsset::new("protected-db", "10.99.0.10", 5432))
            .await
            .unwrap();

        let client_key =
            russh::keys::PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap();
        let client_public_key = client_key.public_key();
        db.add_authorized_key_with_access(
            NewAuthorizedKey::new(
                "contractor-key",
                client_public_key.to_openssh().unwrap(),
                key_fingerprint(client_public_key),
            ),
            AssetAccessMode::Restricted,
            &[],
        )
        .await
        .unwrap();

        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let hop_addr = probe.local_addr().unwrap();
        drop(probe);
        let temp = tempfile::tempdir().unwrap();
        let config = HopConfig {
            listen: hop_addr.to_string(),
            data_dir: temp.path().to_path_buf(),
            ssh: hop_core::config::SshConfig {
                banner: String::new(),
                keepalive_interval: 0,
                connect_timeout: 5,
                ..hop_core::config::SshConfig::default()
            },
            ..HopConfig::default()
        };
        let (_shutdown_tx, shutdown) = watch::channel(false);
        let hop_task = tokio::spawn(serve_ssh_with_shutdown(
            hop_addr,
            config,
            db.clone(),
            master_key,
            ActiveSessionRegistry::default(),
            shutdown,
        ));

        let mut hop_client = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match russh::client::connect(
                    Arc::new(russh::client::Config::default()),
                    hop_addr,
                    AcceptAnyServerKey,
                )
                .await
                {
                    Ok(client) => break Some(client),
                    Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .unwrap()
        .unwrap();
        let hash_alg = hop_client
            .best_supported_rsa_hash()
            .await
            .unwrap()
            .flatten();
        let auth = hop_client
            .authenticate_publickey(
                "proxy",
                PrivateKeyWithHashAlg::new(Arc::new(client_key), hash_alg),
            )
            .await
            .unwrap();
        assert!(auth.success());

        assert!(hop_client
            .channel_open_direct_tcpip("10.99.0.10", 5432, "127.0.0.1", 52100)
            .await
            .is_err());
        assert!(hop_client
            .channel_open_direct_tcpip("10.99.0.99", 9999, "127.0.0.1", 52101)
            .await
            .is_err());

        let sessions = db.list_sessions(10).await.unwrap();
        assert_eq!(sessions.len(), 2);
        let missing = sessions
            .iter()
            .find(|s| s.target_port == Some(9999))
            .unwrap();
        assert_eq!(missing.error.as_deref(), Some("target not found"));
        let unauthorized = sessions
            .iter()
            .find(|s| s.target_port == Some(5432))
            .unwrap();
        assert_eq!(unauthorized.error.as_deref(), Some("target not authorized"));
        assert_eq!(
            unauthorized.asset_id.as_deref(),
            Some(protected.id.as_str())
        );

        let audits = db.list_audit_events(10).await.unwrap();
        assert_eq!(audits.len(), 2);
        assert!(audits
            .iter()
            .any(|a| a.action == "session.access_denied" && a.result == "denied"));
        assert!(audits
            .iter()
            .any(|a| a.action == "session.forward_denied" && a.result == "denied"));

        hop_client
            .disconnect(russh::Disconnect::ByApplication, "done", "en")
            .await
            .ok();
        hop_task.abort();
    }

    #[tokio::test]
    async fn tui_selected_target_enables_cross_connection_and_pending_sftp() {
        use tokio::io::AsyncWriteExt;

        let mut target = ExecTargetServer::default();
        let target_state = target.clone();
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target_listener.local_addr().unwrap();
        let target_config = Arc::new(server::Config {
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![
                russh::keys::PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap(),
            ],
            ..Default::default()
        });
        let target_task =
            tokio::spawn(
                async move { target.run_on_socket(target_config, &target_listener).await },
            );

        let db = Catalog::in_memory().await.unwrap();
        let master_key = Arc::new(MasterKey::from_bytes([21; 32]));
        let credential_id = "kwrt-cred";
        let credential = db
            .add_credential(NewCredential {
                id: Some(credential_id.to_string()),
                name: "kwrt-root".to_string(),
                username: "target-user".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some(
                    encrypt_envelope(
                        &master_key,
                        &format!("{credential_id}:password"),
                        b"target-password",
                    )
                    .unwrap(),
                ),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();
        let mut asset = NewAsset::new(
            "kwrt",
            target_addr.ip().to_string(),
            i64::from(target_addr.port()),
        );
        asset.credential_id = Some(credential.id);
        db.add_asset(asset).await.unwrap();

        let client_key =
            Arc::new(russh::keys::PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap());
        let client_public_key = client_key.public_key();
        db.add_authorized_key(NewAuthorizedKey::new(
            "xterminal-laptop",
            client_public_key.to_openssh().unwrap(),
            key_fingerprint(client_public_key),
        ))
        .await
        .unwrap();

        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let hop_addr = probe.local_addr().unwrap();
        drop(probe);
        let temp = tempfile::tempdir().unwrap();
        let config = HopConfig {
            listen: hop_addr.to_string(),
            data_dir: temp.path().to_path_buf(),
            ssh: hop_core::config::SshConfig {
                banner: String::new(),
                keepalive_interval: 0,
                connect_timeout: 5,
                ..hop_core::config::SshConfig::default()
            },
            ..HopConfig::default()
        };
        let (_shutdown_tx, shutdown) = watch::channel(false);
        let hop_task = tokio::spawn(serve_ssh_with_shutdown(
            hop_addr,
            config,
            db.clone(),
            master_key,
            ActiveSessionRegistry::default(),
            shutdown,
        ));

        #[allow(clippy::async_yields_async)]
        async fn connect_hop_client(
            hop_addr: std::net::SocketAddr,
            client_key: &Arc<russh::keys::PrivateKey>,
        ) -> russh::client::Handle<AcceptAnyServerKey> {
            let mut client = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match russh::client::connect(
                        Arc::new(russh::client::Config::default()),
                        hop_addr,
                        AcceptAnyServerKey,
                    )
                    .await
                    {
                        Ok(client) => break Some(client),
                        Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
                    }
                }
            })
            .await
            .unwrap()
            .unwrap();
            let hash_alg = client.best_supported_rsa_hash().await.unwrap().flatten();
            let auth = client
                .authenticate_publickey(
                    "hop",
                    PrivateKeyWithHashAlg::new(Arc::clone(client_key), hash_alg),
                )
                .await
                .unwrap();
            assert!(auth.success());
            client
        }

        let connect_hop = || connect_hop_client(hop_addr, &client_key);

        // 1. Standalone SFTP to `hop@hop-host` with no TUI open fails fast.
        let standalone = connect_hop().await;
        let mut standalone_ch = standalone.channel_open_session().await.unwrap();
        standalone_ch.request_subsystem(true, "sftp").await.unwrap();
        let standalone_reply =
            tokio::time::timeout(Duration::from_millis(500), standalone_ch.wait())
                .await
                .expect("standalone sftp without open TUI must fail fast");
        assert!(matches!(standalone_reply, Some(ChannelMsg::Failure)));
        standalone
            .disconnect(russh::Disconnect::ByApplication, "done", "en")
            .await
            .ok();

        // 2. Connection 1 opens TUI (`hop@hop-host`); Connection 2 opens SFTP concurrently BEFORE Enter!
        let tui_client = connect_hop().await;
        let mut tui_ch = tui_client.channel_open_session().await.unwrap();
        tui_ch
            .request_pty(true, "xterm-256color", 100, 30, 0, 0, &[])
            .await
            .unwrap();
        tui_ch.request_shell(true).await.unwrap();
        // Drain until TUI screen renders so `ChannelState::Tui` is active.
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(ChannelMsg::Data { .. }) = tui_ch.wait().await {
                    break;
                }
            }
        })
        .await
        .expect("TUI initial render");

        let sftp_client = connect_hop().await;
        let sftp_waiter = tokio::spawn(async move {
            let mut sftp_ch = sftp_client.channel_open_session().await.unwrap();
            sftp_ch.request_subsystem(true, "sftp").await.unwrap();
            loop {
                match sftp_ch.wait().await {
                    Some(ChannelMsg::Success) => break,
                    Some(ChannelMsg::WindowAdjusted { .. }) => continue,
                    other => panic!("expected SFTP subsystem Success, got {other:?}"),
                }
            }
            let mut stream = sftp_ch.into_stream();
            stream.write_all(b"FXP-INIT-PAYLOAD").await.unwrap();
            stream.shutdown().await.unwrap();
            let mut echoed = Vec::new();
            stream.read_to_end(&mut echoed).await.unwrap();
            sftp_client
                .disconnect(russh::Disconnect::ByApplication, "done", "en")
                .await
                .ok();
            echoed
        });

        // Simulate user taking 60ms in TUI before pressing Enter on `kwrt`.
        tokio::time::sleep(Duration::from_millis(60)).await;
        tui_ch.data(&b"\r"[..]).await.unwrap();

        let echoed = tokio::time::timeout(Duration::from_secs(5), sftp_waiter)
            .await
            .expect("concurrent SFTP connection must unblock once TUI selects target")
            .unwrap();
        assert!(
            echoed.starts_with(b"FXP-INIT-PAYLOAD"),
            "unexpected SFTP stream output: {echoed:?}"
        );
        assert_eq!(
            target_state.subsystems.lock().unwrap().as_slice(),
            &["sftp"]
        );

        // Verify an explicit `sftp` session was recorded against `kwrt`.
        let sessions = db.list_sessions(10).await.unwrap();
        assert!(sessions
            .iter()
            .any(|s| s.mode == "sftp" && s.asset_name.as_deref() == Some("kwrt")));

        tui_client
            .disconnect(russh::Disconnect::ByApplication, "done", "en")
            .await
            .ok();
        hop_task.abort();
        target_task.abort();
    }
}
