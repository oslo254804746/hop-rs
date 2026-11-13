use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};

use hop_core::{Asset, Catalog, MasterKey, NewSession};
use russh::{server::Handle, ChannelId, ChannelMsg};
use tokio::sync::{mpsc, watch, Mutex};

use super::{
    client,
    server::{
        accepts_sftp_request, audit_denied_attempt, spawn_tui_termination_watcher,
        start_tui_session, AuthInfo, ChannelState, DenialReason, PtySize, AUTHORIZATION_DENIED,
    },
    session_registry::{
        ActiveSessionRegistry, ResolvedTuiTarget, TuiTargetGuard, TERMINATED_BY_MANAGEMENT,
    },
};
use crate::tui::TuiResources;

pub type SharedChannels = Arc<Mutex<HashMap<ChannelId, ChannelState>>>;
pub type SelectedAssetCleanup = (
    Arc<StdMutex<Option<ResolvedTuiTarget>>>,
    Arc<Mutex<Option<Arc<client::ManagedClientSession>>>>,
);

#[derive(Clone)]
pub struct BridgeControl {
    pub input: mpsc::UnboundedSender<BridgeInput>,
    pub resize: mpsc::UnboundedSender<PtySize>,
    _closed: Arc<watch::Sender<()>>,
}

#[derive(Debug)]
pub enum BridgeInput {
    Data(Vec<u8>),
    Eof,
}

pub(crate) async fn wait_parent_closed(parent_closed: &mut Option<watch::Receiver<bool>>) {
    let Some(rx) = parent_closed.as_mut() else {
        std::future::pending::<()>().await;
        return;
    };
    if *rx.borrow_and_update() {
        return;
    }
    while rx.changed().await.is_ok() {
        if *rx.borrow_and_update() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_session_mode_distinguishes_user_visible_connects() {
        assert_eq!(managed_session_mode(ManagedSessionMode::Tui), "tui-connect");
        assert_eq!(managed_session_mode(ManagedSessionMode::Direct), "direct");
        assert_eq!(managed_session_mode(ManagedSessionMode::Exec), "exec");
        assert_eq!(managed_session_mode(ManagedSessionMode::Sftp), "sftp");
    }
}

pub struct ManagedBridgeOptions {
    pub db: Catalog,
    pub master_key: Arc<MasterKey>,
    pub active_sessions: ActiveSessionRegistry,
    pub auth: AuthInfo,
    pub peer_ip: Option<IpAddr>,
    pub client_ip: Option<String>,
    pub asset: Asset,
    pub channel_id: ChannelId,
    pub handle: Handle,
    pub channels: SharedChannels,
    pub pty: PtySize,
    pub pty_term: String,
    pub pty_requested: bool,
    pub command: Option<Vec<u8>>,
    pub session_mode: ManagedSessionMode,
    pub return_to_tui: Option<(SharedChannels, TuiResources)>,
    pub tui_target_guard: Option<TuiTargetGuard>,
    pub parent_closed: Option<watch::Receiver<bool>>,
    pub selected_asset_cleanup: Option<SelectedAssetCleanup>,
    pub connect_timeout: Duration,
}

pub struct DeferredSftpBridgeOptions {
    pub db: Catalog,
    pub master_key: Arc<MasterKey>,
    pub active_sessions: ActiveSessionRegistry,
    pub auth: AuthInfo,
    pub peer_ip: Option<IpAddr>,
    pub client_ip: Option<String>,
    pub channel_id: ChannelId,
    pub handle: Handle,
    pub channels: SharedChannels,
    pub pty: PtySize,
    pub pty_term: String,
    pub connect_timeout: Duration,
    pub wait_timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedSessionMode {
    Tui,
    Direct,
    Exec,
    Sftp,
}

pub fn spawn_managed_bridge(options: ManagedBridgeOptions) -> BridgeControl {
    let (input_tx, input_rx) = mpsc::unbounded_channel();
    let (resize_tx, resize_rx) = mpsc::unbounded_channel();
    let (closed_tx, client_closed_rx) = watch::channel(());
    tokio::spawn(run_managed_bridge(
        options,
        input_rx,
        resize_rx,
        client_closed_rx,
    ));
    BridgeControl {
        input: input_tx,
        resize: resize_tx,
        _closed: Arc::new(closed_tx),
    }
}

pub fn spawn_deferred_sftp_bridge(options: DeferredSftpBridgeOptions) -> BridgeControl {
    let (input_tx, input_rx) = mpsc::unbounded_channel();
    let (resize_tx, resize_rx) = mpsc::unbounded_channel();
    let (closed_tx, mut client_closed_rx) = watch::channel(());
    tokio::spawn(async move {
        let resolved = tokio::select! {
            _ = client_closed_rx.changed() => return,
            resolved = options
                .active_sessions
                .resolve_tui_target(&options.auth.key_id, options.peer_ip, options.wait_timeout) => resolved,
        };
        let Some(candidate) = resolved else {
            options.channels.lock().await.remove(&options.channel_id);
            let _ = options
                .handle
                .exit_status_request(options.channel_id, 1)
                .await;
            let _ = options.handle.eof(options.channel_id).await;
            let _ = options.handle.close(options.channel_id).await;
            return;
        };
        let fresh = options
            .db
            .get_asset_by_id(&candidate.asset.id)
            .await
            .ok()
            .flatten();
        let Some(asset) = fresh.filter(|a| accepts_sftp_request("sftp", Some(a))) else {
            options.channels.lock().await.remove(&options.channel_id);
            let _ = options
                .handle
                .exit_status_request(options.channel_id, 1)
                .await;
            let _ = options.handle.eof(options.channel_id).await;
            let _ = options.handle.close(options.channel_id).await;
            return;
        };
        if !options
            .db
            .key_can_access_asset(&options.auth.key_id, &asset.id)
            .await
            .unwrap_or(false)
        {
            audit_denied_attempt(
                &options.db,
                &options.auth,
                "sftp",
                DenialReason::TargetNotAuthorized,
                Some(asset.id.clone()),
                Some(asset.name.clone()),
                Some(asset.hostname.clone()),
                Some(asset.port),
                options.client_ip.clone(),
            )
            .await;
            options.channels.lock().await.remove(&options.channel_id);
            let _ = options
                .handle
                .exit_status_request(options.channel_id, 1)
                .await;
            let _ = options.handle.eof(options.channel_id).await;
            let _ = options.handle.close(options.channel_id).await;
            return;
        }
        run_managed_bridge(
            ManagedBridgeOptions {
                db: options.db,
                master_key: options.master_key,
                active_sessions: options.active_sessions,
                auth: options.auth,
                peer_ip: options.peer_ip,
                client_ip: options.client_ip,
                asset,
                channel_id: options.channel_id,
                handle: options.handle,
                channels: options.channels,
                pty: options.pty,
                pty_term: options.pty_term,
                pty_requested: false,
                command: None,
                session_mode: ManagedSessionMode::Sftp,
                return_to_tui: None,
                tui_target_guard: None,
                parent_closed: Some(candidate.parent_closed),
                selected_asset_cleanup: None,
                connect_timeout: options.connect_timeout,
            },
            input_rx,
            resize_rx,
            client_closed_rx,
        )
        .await;
    });
    BridgeControl {
        input: input_tx,
        resize: resize_tx,
        _closed: Arc::new(closed_tx),
    }
}

fn managed_session_mode(mode: ManagedSessionMode) -> &'static str {
    match mode {
        ManagedSessionMode::Tui => "tui-connect",
        ManagedSessionMode::Direct => "direct",
        ManagedSessionMode::Exec => "exec",
        ManagedSessionMode::Sftp => "sftp",
    }
}

async fn run_managed_bridge(
    mut options: ManagedBridgeOptions,
    mut inbound: mpsc::UnboundedReceiver<BridgeInput>,
    mut resize: mpsc::UnboundedReceiver<PtySize>,
    mut client_closed_rx: watch::Receiver<()>,
) {
    let should_return_to_tui = options.return_to_tui.is_some();
    let session = options
        .db
        .start_session(NewSession {
            key_id: Some(options.auth.key_id.clone()),
            key_finger: options.auth.fingerprint.clone(),
            key_name: Some(options.auth.name.clone()),
            mode: managed_session_mode(options.session_mode).to_string(),
            asset_id: Some(options.asset.id.clone()),
            asset_name: Some(options.asset.name.clone()),
            target_host: Some(options.asset.hostname.clone()),
            target_port: Some(options.asset.port),
            client_ip: options.client_ip.clone(),
        })
        .await;
    let session_id = session.as_ref().map(|s| s.id.clone()).ok();
    let (terminate_tx, mut terminate) = mpsc::unbounded_channel();
    if let Some(session_id) = &session_id {
        options
            .active_sessions
            .register(session_id.clone(), terminate_tx)
            .await;
    }

    let mut terminated = false;
    let mut parent_closed = false;
    let mut client_closed = false;
    let mut exit_reported = false;
    let result = async {
        if !options
            .db
            .key_can_access_asset(&options.auth.key_id, &options.asset.id)
            .await?
        {
            anyhow::bail!(AUTHORIZATION_DENIED);
        }
        let is_sftp = options.session_mode == ManagedSessionMode::Sftp;
        let is_exec = options.session_mode == ManagedSessionMode::Exec;
        if !is_sftp && !is_exec {
            let _ = options
                .handle
                .data(options.channel_id, b"\r\n\x1b[2J\x1b[HConnecting to target...\r\n".to_vec())
                .await;
        }
        let command = if is_exec {
            let Some(command) = options.command.take() else {
                anyhow::bail!("managed exec session is missing its command");
            };
            Some(command)
        } else {
            None
        };
        let connect_started = Instant::now();
        let target_result = if is_sftp {
            tokio::select! {
                Some(()) = terminate.recv() => {
                    terminated = true;
                    return Ok(());
                }
                () = wait_parent_closed(&mut options.parent_closed) => {
                    parent_closed = true;
                    return Ok(());
                }
                _ = client_closed_rx.changed() => {
                    client_closed = true;
                    return Ok(());
                }
                target = client::connect_asset_sftp(
                    options.db.clone(),
                    options.master_key.clone(),
                    &options.asset,
                    options.connect_timeout,
                ) => target
            }
        } else if is_exec {
            let pty = options.pty_requested.then_some((
                options.pty_term.as_str(),
                options.pty.width as u32,
                options.pty.height as u32,
            ));
            tokio::select! {
                Some(()) = terminate.recv() => {
                    terminated = true;
                    return Ok(());
                }
                () = wait_parent_closed(&mut options.parent_closed) => {
                    parent_closed = true;
                    return Ok(());
                }
                _ = client_closed_rx.changed() => {
                    client_closed = true;
                    return Ok(());
                }
                target = client::connect_asset_exec(
                    options.db.clone(),
                    options.master_key.clone(),
                    &options.asset,
                    command.expect("exec command checked above"),
                    pty,
                    options.connect_timeout,
                ) => target
            }
        } else {
            tokio::select! {
                Some(()) = terminate.recv() => {
                    terminated = true;
                    return Ok(());
                }
                () = wait_parent_closed(&mut options.parent_closed) => {
                    parent_closed = true;
                    return Ok(());
                }
                _ = client_closed_rx.changed() => {
                    client_closed = true;
                    return Ok(());
                }
                target = client::connect_asset_shell(
                    options.db.clone(),
                    options.master_key.clone(),
                    &options.asset,
                    &options.pty_term,
                    options.pty.width as u32,
                    options.pty.height as u32,
                    options.connect_timeout,
                ) => target
            }
        };
        let mut target = match target_result {
            Ok(target) => {
                let latency_ms = connect_started
                    .elapsed()
                    .as_millis()
                    .min(i64::MAX as u128) as i64;
                let _ = options
                    .db
                    .record_asset_health_success(&options.asset.id, Some(latency_ms))
                    .await;
                target
            }
            Err(err) => {
                let message = health_error_message(&err);
                if !(is_sftp && message.contains("sftp subsystem")) {
                    let _ = options
                        .db
                        .record_asset_health_failure(
                            &options.asset.id,
                            Some("managed_connect_failed"),
                            Some(&message),
                        )
                        .await;
                }
                return Err(err);
            }
        };

        loop {
            tokio::select! {
                Some(()) = terminate.recv() => {
                    terminated = true;
                    let message = format!("{TERMINATED_BY_MANAGEMENT}\n").into_bytes();
                    if is_exec {
                        let _ = options
                            .handle
                            .extended_data(options.channel_id, 1, message)
                            .await;
                    } else {
                        let _ = options
                            .handle
                            .data(
                                options.channel_id,
                                format!("\r\n{TERMINATED_BY_MANAGEMENT}\r\n").into_bytes(),
                            )
                            .await;
                    }
                    break;
                }
                () = wait_parent_closed(&mut options.parent_closed) => {
                    parent_closed = true;
                    break;
                }
                input = inbound.recv() => {
                    match input {
                        Some(BridgeInput::Data(data)) => target.channel.data_bytes(data).await?,
                        Some(BridgeInput::Eof) => target.channel.eof().await?,
                        None => {
                            client_closed = true;
                            break;
                        }
                    }
                }
                Some(size) = resize.recv(), if !is_sftp && (!is_exec || options.pty_requested) => {
                    target.channel.window_change(size.width as u32, size.height as u32, 0, 0).await?;
                }
                msg = target.channel.wait() => {
                    match msg {
                        Some(ChannelMsg::Data { data }) => {
                            let _ = options.handle.data(options.channel_id, data.to_vec()).await;
                        }
                        Some(ChannelMsg::ExtendedData { data, ext: 1 }) => {
                            let _ = options.handle.extended_data(options.channel_id, 1, data.to_vec()).await;
                        }
                        Some(ChannelMsg::ExitStatus { exit_status }) => {
                            if !should_return_to_tui {
                                let _ = options.handle.exit_status_request(options.channel_id, exit_status).await;
                                exit_reported = true;
                            }
                        }
                        Some(ChannelMsg::ExitSignal {
                            signal_name,
                            core_dumped,
                            error_message,
                            lang_tag,
                        }) => {
                            if !should_return_to_tui {
                                let _ = options
                                    .handle
                                    .exit_signal_request(
                                        options.channel_id,
                                        signal_name,
                                        core_dumped,
                                        error_message,
                                        lang_tag,
                                    )
                                    .await;
                                exit_reported = true;
                            }
                        }
                        Some(ChannelMsg::Eof) => {}
                        Some(ChannelMsg::Close) | None => break,
                        _ => {}
                    }
                }
            }
        }
        let reason = if terminated {
            TERMINATED_BY_MANAGEMENT
        } else if parent_closed {
            "tui target closed"
        } else if client_closed {
            "client disconnected"
        } else {
            "target closed"
        };
        target.client.disconnect(reason).await;
        if is_exec && !exit_reported && !terminated && !parent_closed && !client_closed {
            anyhow::bail!("target closed without reporting an exit status");
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    if let Some((selected_slot, direct_client)) = options.selected_asset_cleanup.take() {
        if let Ok(mut guard) = selected_slot.lock() {
            *guard = None;
        }
        if let Some(cached) = direct_client.lock().await.take() {
            cached.disconnect("tui target closed").await;
        }
    }

    if let Some(session_id) = session_id {
        options.active_sessions.unregister(&session_id).await;
        let _ = match &result {
            _ if terminated => {
                options
                    .db
                    .finish_session(&session_id, "terminated", Some(TERMINATED_BY_MANAGEMENT))
                    .await
            }
            Ok(_) => options.db.finish_session(&session_id, "ok", None).await,
            Err(err) => {
                options
                    .db
                    .finish_session(&session_id, "failed", Some(&err.to_string()))
                    .await
            }
        };
    }

    if terminated {
        options.channels.lock().await.remove(&options.channel_id);
        drop(options.tui_target_guard.take());
        let _ = options
            .handle
            .exit_status_request(options.channel_id, 255)
            .await;
        let _ = options.handle.eof(options.channel_id).await;
        let _ = options.handle.close(options.channel_id).await;
    } else if let Some((channels, mut tui)) = options.return_to_tui.take() {
        if client_closed {
            drop(options.tui_target_guard.take());
            return;
        }
        let reopened_selector_guard = options
            .active_sessions
            .register_tui_selector(&options.auth.key_id, options.peer_ip);
        drop(options.tui_target_guard.take());
        let message = match &result {
            Ok(_) => "\r\nTarget session ended. Returning to Hop...\r\n",
            Err(err) => {
                let msg = format!("\r\nTarget connection failed: {err}\r\nReturning to Hop...\r\n");
                let _ = options
                    .handle
                    .data(options.channel_id, msg.into_bytes())
                    .await;
                ""
            }
        };
        if !message.is_empty() {
            let _ = options
                .handle
                .data(options.channel_id, message.as_bytes().to_vec())
                .await;
        }
        let should_return = channels.lock().await.contains_key(&options.channel_id);
        if should_return {
            if let Ok(fresh_assets) = options.db.list_assets_for_key(&options.auth.key_id).await {
                tui.set_assets(fresh_assets);
            }
            match start_tui_session(&options.db, &options.auth, options.client_ip.clone()).await {
                Ok(audit) => match tui.resume_after_target() {
                    Ok(output) => {
                        if options
                            .handle
                            .data(options.channel_id, output)
                            .await
                            .is_ok()
                        {
                            let reopened_session_id = audit.id().to_string();
                            let (reopen_tx, reopen_rx) = mpsc::unbounded_channel();
                            channels.lock().await.insert(
                                options.channel_id,
                                ChannelState::Tui {
                                    tui: Box::new(tui),
                                    audit,
                                    selector_guard: reopened_selector_guard,
                                },
                            );
                            spawn_tui_termination_watcher(
                                options.db.clone(),
                                options.active_sessions.clone(),
                                reopened_session_id.clone(),
                                channels.clone(),
                                options.channel_id,
                                options.handle.clone(),
                                reopen_rx,
                            );
                            options
                                .active_sessions
                                .register(reopened_session_id, reopen_tx)
                                .await;
                        } else {
                            drop(reopened_selector_guard);
                            let error = "ssh channel closed while reopening Hop TUI";
                            let _ = audit.finish(&options.db, "failed", Some(error)).await;
                            let _ = options.handle.eof(options.channel_id).await;
                            let _ = options.handle.close(options.channel_id).await;
                        }
                    }
                    Err(err) => {
                        drop(reopened_selector_guard);
                        let error = err.to_string();
                        let _ = audit.finish(&options.db, "failed", Some(&error)).await;
                        let _ = options
                            .handle
                            .data(
                                options.channel_id,
                                format!("\r\nFailed to reopen Hop TUI: {error}\r\n").into_bytes(),
                            )
                            .await;
                        let _ = options.handle.eof(options.channel_id).await;
                        let _ = options.handle.close(options.channel_id).await;
                    }
                },
                Err(err) => {
                    drop(reopened_selector_guard);
                    let _ = options
                        .handle
                        .data(
                            options.channel_id,
                            format!("\r\nFailed to reopen Hop TUI: {err}\r\n").into_bytes(),
                        )
                        .await;
                    let _ = options.handle.eof(options.channel_id).await;
                    let _ = options.handle.close(options.channel_id).await;
                }
            }
        } else {
            drop(reopened_selector_guard);
        }
    } else {
        options.channels.lock().await.remove(&options.channel_id);
        drop(options.tui_target_guard.take());
        if !client_closed {
            if let Err(err) = &result {
                if options.session_mode == ManagedSessionMode::Sftp {
                    let _ = options
                        .handle
                        .exit_status_request(options.channel_id, 1)
                        .await;
                } else if options.session_mode == ManagedSessionMode::Exec {
                    if !exit_reported {
                        let _ = options
                            .handle
                            .extended_data(
                                options.channel_id,
                                1,
                                format!("hop: target connection failed: {err}\n").into_bytes(),
                            )
                            .await;
                        let _ = options
                            .handle
                            .exit_status_request(options.channel_id, 255)
                            .await;
                    }
                } else {
                    let _ = options
                        .handle
                        .data(
                            options.channel_id,
                            format!("\r\ndirect connection failed: {err}\r\n").into_bytes(),
                        )
                        .await;
                }
            } else if parent_closed && !exit_reported {
                let _ = options
                    .handle
                    .exit_status_request(options.channel_id, 0)
                    .await;
            }
            let _ = options.handle.eof(options.channel_id).await;
            let _ = options.handle.close(options.channel_id).await;
        }
    }
}

fn health_error_message(err: &impl std::fmt::Display) -> String {
    err.to_string().chars().take(512).collect()
}
