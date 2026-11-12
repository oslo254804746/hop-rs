use std::{sync::Arc, time::Duration};

use anyhow::Result;
use hop_core::{
    Catalog, HopConfig, MasterKey, NewAuditEvent, NotificationChannel, ProbeSettings,
    ASSET_HEALTH_FAILED, ASSET_HEALTH_HEALTHY,
};
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::notify::{send_channel_notification, AlertEvent};

#[derive(Debug, Clone, Default)]
pub struct ProbeCycleSummary {
    pub probed_assets: usize,
    pub healthy_assets: usize,
    pub alerts_fired: usize,
    pub recoveries_sent: usize,
    pub delivery_errors: usize,
}

pub async fn sync_startup_probe_and_notifications(
    db: &Catalog,
    master_key: &MasterKey,
    config: &HopConfig,
) -> Result<()> {
    if let Some(startup_probe) = &config.probe {
        let desired = ProbeSettings::from(startup_probe);
        db.update_probe_settings(&desired).await?;
    }

    if !config.notifications.is_empty() {
        let existing = db.list_notification_channels().await?;
        for (name, spec) in &config.notifications {
            let existing_id = existing
                .iter()
                .find(|chan| &chan.name == name)
                .map(|chan| chan.id.clone());
            let built = spec.to_encrypted_channel(existing_id.clone(), name, master_key)?;
            if let Some(id) = existing_id {
                db.update_notification_channel(&id, built).await?;
            } else {
                db.add_notification_channel(built).await?;
            }
        }
    }
    Ok(())
}

pub async fn run_probe_cycle_and_evaluate_alerts(
    db: &Catalog,
    master_key: Arc<MasterKey>,
    settings: &ProbeSettings,
) -> Result<ProbeCycleSummary> {
    let timeout = Duration::from_millis(settings.timeout_ms.clamp(500, 15_000) as u64);
    let refreshed =
        crate::probe::probe_all_assets(db, Some(Arc::clone(&master_key)), timeout).await?;
    if refreshed.is_empty() {
        return Ok(ProbeCycleSummary::default());
    }

    let enabled_channels: Vec<NotificationChannel> = db
        .list_notification_channels()
        .await?
        .into_iter()
        .filter(|chan| chan.enabled)
        .collect();

    let mut summary = ProbeCycleSummary {
        probed_assets: refreshed.len(),
        healthy_assets: refreshed
            .iter()
            .filter(|a| a.health.status == ASSET_HEALTH_HEALTHY)
            .count(),
        ..ProbeCycleSummary::default()
    };

    let threshold = settings.failure_threshold.max(1);
    for asset in &refreshed {
        if asset.health.status == ASSET_HEALTH_FAILED
            && !asset.health.alert_firing
            && asset.health.consecutive_failures >= threshold
        {
            db.mark_asset_alert_firing(&asset.id, true).await?;
            summary.alerts_fired += 1;
            let event = AlertEvent::from_down_asset(asset);
            let errors = dispatch_event(db, &master_key, &enabled_channels, &event).await;
            summary.delivery_errors += errors;

            let _ = db
                .record_audit_event(NewAuditEvent {
                    actor_label: "probe-scheduler".to_string(),
                    action: "probe.alert_fired".to_string(),
                    target_type: "asset".to_string(),
                    target_id: Some(asset.id.clone()),
                    target_label: Some(asset.name.clone()),
                    result: if errors == 0 {
                        "success".to_string()
                    } else {
                        "partial_failure".to_string()
                    },
                    source_ip: None,
                    details_json: Some(
                        serde_json::json!({
                            "consecutive_failures": asset.health.consecutive_failures,
                            "error_code": asset.health.error_code,
                            "channels": enabled_channels.len(),
                            "delivery_errors": errors,
                        })
                        .to_string(),
                    ),
                })
                .await;
        } else if asset.health.status == ASSET_HEALTH_HEALTHY && asset.health.alert_firing {
            db.mark_asset_alert_firing(&asset.id, false).await?;
            let mut errors = 0usize;
            if settings.notify_on_recovery {
                summary.recoveries_sent += 1;
                let event = AlertEvent::from_recovered_asset(asset);
                errors = dispatch_event(db, &master_key, &enabled_channels, &event).await;
                summary.delivery_errors += errors;
            }
            let _ = db
                .record_audit_event(NewAuditEvent {
                    actor_label: "probe-scheduler".to_string(),
                    action: "probe.alert_recovered".to_string(),
                    target_type: "asset".to_string(),
                    target_id: Some(asset.id.clone()),
                    target_label: Some(asset.name.clone()),
                    result: if errors == 0 {
                        "success".to_string()
                    } else {
                        "partial_failure".to_string()
                    },
                    source_ip: None,
                    details_json: Some(
                        serde_json::json!({
                            "latency_ms": asset.health.latency_ms,
                            "notified": settings.notify_on_recovery,
                            "delivery_errors": errors,
                        })
                        .to_string(),
                    ),
                })
                .await;
        }
    }

    Ok(summary)
}

async fn dispatch_event(
    db: &Catalog,
    master_key: &MasterKey,
    channels: &[NotificationChannel],
    event: &AlertEvent,
) -> usize {
    let mut errors = 0usize;
    for channel in channels {
        if let Err(err) = send_channel_notification(master_key, channel, event).await {
            errors += 1;
            warn!(
                channel_id = %channel.id,
                channel_name = %channel.name,
                kind = %channel.kind,
                %err,
                "scheduled alert notification delivery failed"
            );
            let _ = db
                .record_audit_event(NewAuditEvent {
                    actor_label: "probe-scheduler".to_string(),
                    action: "notification.delivery_failed".to_string(),
                    target_type: "notification_channel".to_string(),
                    target_id: Some(channel.id.clone()),
                    target_label: Some(channel.name.clone()),
                    result: "failed".to_string(),
                    source_ip: None,
                    details_json: Some(
                        serde_json::json!({
                            "kind": channel.kind,
                            "error": err.to_string(),
                        })
                        .to_string(),
                    ),
                })
                .await;
        }
    }
    errors
}

pub async fn serve_probe_scheduler_with_shutdown(
    db: Catalog,
    master_key: Arc<MasterKey>,
    mut reload_rx: watch::Receiver<u64>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut was_enabled = false;
    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        let settings = match db.get_probe_settings().await {
            Ok(settings) => settings,
            Err(err) => {
                warn!(%err, "failed loading probe_settings; retrying in 30s");
                tokio::select! {
                    _ = shutdown_rx.changed() => break,
                    _ = tokio::time::sleep(Duration::from_secs(30)) => continue,
                }
            }
        };

        if !settings.enabled {
            was_enabled = false;
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
                res = reload_rx.changed() => {
                    if res.is_err() {
                        break;
                    }
                }
            }
            continue;
        }

        if !was_enabled {
            was_enabled = true;
            info!(
                interval_seconds = settings.interval_seconds,
                failure_threshold = settings.failure_threshold,
                "background probe scheduler enabled; running initial sweep"
            );
            if let Err(err) =
                run_probe_cycle_and_evaluate_alerts(&db, Arc::clone(&master_key), &settings).await
            {
                warn!(%err, "initial scheduled probe cycle failed");
            }
        }

        let sleep_secs = settings.interval_seconds.clamp(30, 86_400) as u64;
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            res = reload_rx.changed() => {
                if res.is_err() {
                    break;
                }
                debug!("probe scheduler reloaded settings");
                continue;
            }
            _ = tokio::time::sleep(Duration::from_secs(sleep_secs)) => {
                let latest = db.get_probe_settings().await.unwrap_or(settings);
                if latest.enabled {
                    match run_probe_cycle_and_evaluate_alerts(&db, Arc::clone(&master_key), &latest).await {
                        Ok(summary) => {
                            debug!(
                                probed = summary.probed_assets,
                                healthy = summary.healthy_assets,
                                alerts = summary.alerts_fired,
                                recoveries = summary.recoveries_sent,
                                "scheduled probe cycle completed"
                            );
                        }
                        Err(err) => {
                            warn!(%err, "scheduled probe cycle failed");
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use hop_core::{
        Catalog, MasterKey, NewAsset, NewNotificationChannel, NotificationChannelKind,
        ProbeSettings, ASSET_PROTOCOL_TCP,
    };
    use tokio::{io::AsyncWriteExt, net::TcpListener};

    use super::*;
    use crate::notify::{
        build_encrypted_notification_config, tests::read_http_request_for_test,
        NotificationConfigInput,
    };

    #[tokio::test]
    async fn probe_cycle_fires_alert_at_threshold_suppresses_duplicates_and_notifies_on_recovery() {
        let webhook_hits = Arc::new(AtomicUsize::new(0));
        let webhook_hits_bg = Arc::clone(&webhook_hits);
        let wh_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let wh_addr = wh_listener.local_addr().unwrap();
        let wh_task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = wh_listener.accept().await {
                let hits = Arc::clone(&webhook_hits_bg);
                tokio::spawn(async move {
                    let _ = read_http_request_for_test(&mut stream).await;
                    hits.fetch_add(1, Ordering::SeqCst);
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )
                        .await;
                    let _ = stream.shutdown().await;
                });
            }
        });

        let probe_sock = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let target_addr = probe_sock.local_addr().unwrap();
        drop(probe_sock);

        let db = Catalog::in_memory().await.unwrap();
        let master_key = Arc::new(MasterKey::from_bytes([41; 32]));

        let chan_id = "wh-alert-1";
        let mut wh_input = NotificationConfigInput::default();
        wh_input.url = Some(format!("http://127.0.0.1:{}/alert", wh_addr.port()));
        let config_enc = build_encrypted_notification_config(
            &master_key,
            chan_id,
            NotificationChannelKind::Webhook,
            &wh_input,
            None,
        )
        .unwrap();
        db.add_notification_channel(NewNotificationChannel {
            id: Some(chan_id.to_string()),
            name: "ops-webhook".to_string(),
            kind: NotificationChannelKind::Webhook,
            enabled: true,
            config_enc,
        })
        .await
        .unwrap();

        let asset = db
            .add_asset(NewAsset {
                name: "jp-edge".to_string(),
                protocol: ASSET_PROTOCOL_TCP.to_string(),
                hostname: target_addr.ip().to_string(),
                port: i64::from(target_addr.port()),
                display_name: None,
                description: None,
                tags: Vec::new(),
                credential_id: None,
                via_asset_id: None,
                allow_tcp_forwarding: true,
            })
            .await
            .unwrap();

        let settings = db
            .update_probe_settings(&ProbeSettings {
                enabled: true,
                interval_seconds: 60,
                timeout_ms: 800,
                failure_threshold: 2,
                notify_on_recovery: true,
                updated_at: None,
            })
            .await
            .unwrap();

        let c1 = run_probe_cycle_and_evaluate_alerts(&db, Arc::clone(&master_key), &settings)
            .await
            .unwrap();
        assert_eq!(c1.alerts_fired, 0);
        assert_eq!(webhook_hits.load(Ordering::SeqCst), 0);

        let c2 = run_probe_cycle_and_evaluate_alerts(&db, Arc::clone(&master_key), &settings)
            .await
            .unwrap();
        assert_eq!(c2.alerts_fired, 1);
        assert_eq!(webhook_hits.load(Ordering::SeqCst), 1);
        assert!(
            db.get_asset_by_id(&asset.id)
                .await
                .unwrap()
                .unwrap()
                .health
                .alert_firing
        );

        let c3 = run_probe_cycle_and_evaluate_alerts(&db, Arc::clone(&master_key), &settings)
            .await
            .unwrap();
        assert_eq!(c3.alerts_fired, 0);
        assert_eq!(webhook_hits.load(Ordering::SeqCst), 1);

        let recovered_listener = TcpListener::bind(target_addr).await.unwrap();
        let target_task =
            tokio::spawn(
                async move { while let Ok((_sock, _)) = recovered_listener.accept().await {} },
            );

        let c4 = run_probe_cycle_and_evaluate_alerts(&db, Arc::clone(&master_key), &settings)
            .await
            .unwrap();
        assert_eq!(c4.recoveries_sent, 1);
        assert_eq!(webhook_hits.load(Ordering::SeqCst), 2);
        assert!(
            !db.get_asset_by_id(&asset.id)
                .await
                .unwrap()
                .unwrap()
                .health
                .alert_firing
        );

        wh_task.abort();
        target_task.abort();
    }
}
