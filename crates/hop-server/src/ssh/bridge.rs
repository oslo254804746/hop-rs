use std::{collections::HashMap, sync::Arc};

use hop_core::{Asset, HopDb, MasterKey, NewSession};
use russh::{server::Handle, ChannelId, ChannelMsg};
use tokio::sync::{mpsc, Mutex};

use super::{
    client,
    server::{start_tui_session, AuthInfo, ChannelState, PtySize},
};
use crate::tui::TuiResources;

pub type SharedChannels = Arc<Mutex<HashMap<ChannelId, ChannelState>>>;

#[derive(Clone)]
pub struct BridgeControl {
    pub input: mpsc::UnboundedSender<Vec<u8>>,
    pub resize: mpsc::UnboundedSender<PtySize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_session_mode_distinguishes_tui_connects() {
        assert_eq!(managed_session_mode(ManagedSessionMode::Tui), "tui-connect");
        assert_eq!(
            managed_session_mode(ManagedSessionMode::Exec),
            "exec-connect"
        );
        assert_eq!(managed_session_mode(ManagedSessionMode::Direct), "direct");
    }
}

pub struct ManagedBridgeOptions {
    pub db: HopDb,
    pub master_key: Arc<MasterKey>,
    pub auth: AuthInfo,
    pub client_ip: Option<String>,
    pub asset: Asset,
    pub channel_id: ChannelId,
    pub handle: Handle,
    pub pty: PtySize,
    pub session_mode: ManagedSessionMode,
    pub return_to_tui: Option<(SharedChannels, TuiResources)>,
    pub connect_timeout: std::time::Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedSessionMode {
    Tui,
    Exec,
    Direct,
}

pub fn spawn_managed_bridge(options: ManagedBridgeOptions) -> BridgeControl {
    let (input_tx, input_rx) = mpsc::unbounded_channel();
    let (resize_tx, resize_rx) = mpsc::unbounded_channel();
    tokio::spawn(run_managed_bridge(options, input_rx, resize_rx));
    BridgeControl {
        input: input_tx,
        resize: resize_tx,
    }
}

fn managed_session_mode(mode: ManagedSessionMode) -> &'static str {
    match mode {
        ManagedSessionMode::Tui => "tui-connect",
        ManagedSessionMode::Exec => "exec-connect",
        ManagedSessionMode::Direct => "direct",
    }
}

async fn run_managed_bridge(
    mut options: ManagedBridgeOptions,
    mut inbound: mpsc::UnboundedReceiver<Vec<u8>>,
    mut resize: mpsc::UnboundedReceiver<PtySize>,
) {
    let should_return_to_tui = options.return_to_tui.is_some();
    let session = options
        .db
        .start_session(NewSession {
            key_finger: options.auth.fingerprint.clone(),
            key_name: Some(options.auth.name.clone()),
            mode: managed_session_mode(options.session_mode).to_string(),
            asset_name: Some(options.asset.name.clone()),
            target_host: Some(options.asset.hostname.clone()),
            target_port: Some(options.asset.port),
            client_ip: options.client_ip.clone(),
        })
        .await;
    let session_id = session.as_ref().map(|s| s.id.clone()).ok();

    let result = async {
        let _ = options
            .handle
            .data(options.channel_id, b"\r\n\x1b[2J\x1b[HConnecting to target...\r\n".to_vec())
            .await;
        let mut target = client::connect_asset_shell(
            options.db.clone(),
            options.master_key.clone(),
            &options.asset,
            options.pty.width as u32,
            options.pty.height as u32,
            options.connect_timeout,
        )
        .await?;

        loop {
            tokio::select! {
                Some(data) = inbound.recv() => {
                    target.channel.data_bytes(data).await?;
                }
                Some(size) = resize.recv() => {
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
                            }
                            break;
                        }
                        Some(ChannelMsg::Eof | ChannelMsg::Close) | None => break,
                        _ => {}
                    }
                }
            }
        }
        target.session.disconnect(russh::Disconnect::ByApplication, "target closed", "en").await.ok();
        Ok::<(), anyhow::Error>(())
    }
    .await;

    if let Some(session_id) = session_id {
        let _ = match &result {
            Ok(_) => options.db.finish_session(&session_id, "ok", None).await,
            Err(err) => {
                options
                    .db
                    .finish_session(&session_id, "failed", Some(&err.to_string()))
                    .await
            }
        };
    }

    if let Some((channels, mut tui)) = options.return_to_tui.take() {
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
            match start_tui_session(&options.db, &options.auth, options.client_ip.clone()).await {
                Ok(audit) => match tui.resume_after_target() {
                    Ok(output) => {
                        if options
                            .handle
                            .data(options.channel_id, output)
                            .await
                            .is_ok()
                        {
                            channels.lock().await.insert(
                                options.channel_id,
                                ChannelState::Tui {
                                    tui: Box::new(tui),
                                    audit,
                                },
                            );
                        } else {
                            let error = "ssh channel closed while reopening Hop TUI";
                            let _ = audit.finish(&options.db, "failed", Some(error)).await;
                            let _ = options.handle.eof(options.channel_id).await;
                            let _ = options.handle.close(options.channel_id).await;
                        }
                    }
                    Err(err) => {
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
        }
    } else {
        if let Err(err) = &result {
            let _ = options
                .handle
                .data(
                    options.channel_id,
                    format!("\r\nhop-connect failed: {err}\r\n").into_bytes(),
                )
                .await;
        }
        let _ = options.handle.eof(options.channel_id).await;
        let _ = options.handle.close(options.channel_id).await;
    }
}
