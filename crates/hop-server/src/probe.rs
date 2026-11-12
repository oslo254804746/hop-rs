use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use hop_core::{validate_tcp_port, Asset, Catalog, MasterKey, ASSET_PROTOCOL_SSH};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::Semaphore,
    task::JoinSet,
};

const MIN_PROBE_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_DIRECT_PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CHAINED_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONCURRENT_PROBES: usize = 16;
const MAX_BANNER_LINES: usize = 8;
const MAX_BANNER_LINE_BYTES: usize = 256;

pub async fn probe_asset(
    db: &Catalog,
    master_key: Option<&MasterKey>,
    asset: &Asset,
    timeout: Duration,
) -> Result<Asset> {
    let max_timeout = if asset.via_asset_id.is_some() {
        MAX_CHAINED_PROBE_TIMEOUT
    } else {
        MAX_DIRECT_PROBE_TIMEOUT
    };
    let bounded = timeout.clamp(MIN_PROBE_TIMEOUT, max_timeout);
    match run_network_probe(db, master_key, asset, bounded).await {
        Ok(latency_ms) => {
            db.record_asset_health_success(&asset.id, Some(latency_ms))
                .await?;
            let _ = db
                .record_probe_sample(&asset.id, true, Some(latency_ms), None)
                .await;
        }
        Err((code, message)) => {
            let truncated: String = message.chars().take(512).collect();
            db.record_asset_health_failure(&asset.id, Some(code), Some(&truncated))
                .await?;
            let _ = db
                .record_probe_sample(&asset.id, false, None, Some(code))
                .await;
        }
    }

    db.get_asset_by_id(&asset.id)
        .await?
        .with_context(|| format!("asset disappeared during probe: {}", asset.id))
}

pub async fn probe_assets(
    db: &Catalog,
    master_key: Option<Arc<MasterKey>>,
    assets: &[Asset],
    timeout: Duration,
) -> Result<Vec<Asset>> {
    if assets.is_empty() {
        return Ok(Vec::new());
    }
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_PROBES));
    let mut join_set = JoinSet::new();
    for asset in assets {
        let asset = asset.clone();
        let db = db.clone();
        let master_key = master_key.clone();
        let semaphore = Arc::clone(&semaphore);
        join_set.spawn(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .context("probe semaphore closed")?;
            probe_asset(&db, master_key.as_deref(), &asset, timeout).await
        });
    }

    let mut refreshed = Vec::with_capacity(assets.len());
    while let Some(joined) = join_set.join_next().await {
        refreshed.push(joined.context("probe task panicked")??);
    }
    refreshed.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(refreshed)
}

pub async fn probe_all_assets(
    db: &Catalog,
    master_key: Option<Arc<MasterKey>>,
    timeout: Duration,
) -> Result<Vec<Asset>> {
    let assets = db.list_assets().await?;
    probe_assets(db, master_key, &assets, timeout).await
}

async fn run_network_probe(
    db: &Catalog,
    master_key: Option<&MasterKey>,
    asset: &Asset,
    timeout: Duration,
) -> std::result::Result<i64, (&'static str, String)> {
    let port = validate_tcp_port(asset.port).map_err(|err| ("invalid_port", err.to_string()))?;
    let started = Instant::now();

    if asset.via_asset_id.is_some() {
        let stream = match tokio::time::timeout(
            timeout,
            crate::ssh::client::open_asset_stream(db, master_key, asset, timeout),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(err)) => {
                let text = format!("{err:#}");
                let code = if text.contains("timed out") {
                    "probe_timeout"
                } else if text.contains("jump forward via") {
                    "tcp_connect_failed"
                } else {
                    "via_jump_failed"
                };
                return Err((code, text));
            }
            Err(_) => {
                return Err((
                    "probe_timeout",
                    format!(
                        "chained probe {}:{} timed out after {}ms",
                        asset.hostname,
                        port,
                        timeout.as_millis()
                    ),
                ));
            }
        };

        return finish_probe_over_stream(asset, port, started, timeout, stream).await;
    }

    let stream =
        match tokio::time::timeout(timeout, TcpStream::connect((asset.hostname.as_str(), port)))
            .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(err)) => {
                return Err((
                    "tcp_connect_failed",
                    format!("connect {}:{}: {err}", asset.hostname, port),
                ));
            }
            Err(_) => {
                return Err((
                    "probe_timeout",
                    format!(
                        "connect {}:{} timed out after {}ms",
                        asset.hostname,
                        port,
                        timeout.as_millis()
                    ),
                ));
            }
        };

    let _ = stream.set_nodelay(true);
    finish_probe_over_stream(asset, port, started, timeout, stream).await
}

async fn finish_probe_over_stream<S>(
    asset: &Asset,
    port: u16,
    started: Instant,
    timeout: Duration,
    mut stream: S,
) -> std::result::Result<i64, (&'static str, String)>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if asset.protocol == ASSET_PROTOCOL_SSH {
        let remaining = timeout
            .saturating_sub(started.elapsed())
            .max(Duration::from_millis(250));
        match tokio::time::timeout(remaining, read_ssh_banner(stream)).await {
            Ok(Ok(())) => {}
            Ok(Err(message)) => return Err(("ssh_banner_invalid", message)),
            Err(_) => {
                return Err((
                    "ssh_banner_timeout",
                    format!(
                        "{}:{} accepted TCP but sent no SSH banner within {}ms",
                        asset.hostname,
                        port,
                        timeout.as_millis()
                    ),
                ));
            }
        }
    } else {
        let _ = stream.shutdown().await;
    }

    let latency_ms = started.elapsed().as_millis().min(i64::MAX as u128) as i64;
    Ok(latency_ms)
}

async fn read_ssh_banner<S>(stream: S) -> std::result::Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(stream);
    let mut line = Vec::with_capacity(64);
    for _ in 0..MAX_BANNER_LINES {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .await
            .map_err(|err| format!("failed reading SSH banner: {err}"))?;
        if read == 0 {
            return Err("connection closed before sending SSH protocol banner".to_string());
        }
        if line.len() > MAX_BANNER_LINE_BYTES && !line.starts_with(b"SSH-") {
            return Err("received non-SSH banner from target port".to_string());
        }
        if line.starts_with(b"SSH-") {
            let mut stream = reader.into_inner();
            let _ = stream.shutdown().await;
            return Ok(());
        }
    }
    Err("target did not present an SSH-2.0 banner".to_string())
}

#[cfg(test)]
mod tests {
    use hop_core::{NewAsset, ASSET_PROTOCOL_TCP};
    use tokio::{io::AsyncWriteExt, net::TcpListener};

    use super::*;

    #[tokio::test]
    async fn ssh_probe_records_healthy_latency_when_banner_is_present() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = socket.write_all(b"SSH-2.0-OpenSSH_9.6\r\n").await;
            }
        });

        let catalog = Catalog::in_memory().await.unwrap();
        let asset = catalog
            .add_asset(NewAsset::new(
                "hk-vps",
                addr.ip().to_string(),
                i64::from(addr.port()),
            ))
            .await
            .unwrap();

        let probed = probe_asset(&catalog, None, &asset, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(probed.health.status, "healthy");
        assert!(probed.health.latency_ms.is_some());
        assert!(probed.health.error_code.is_none());

        let samples = catalog
            .list_asset_probe_samples(&asset.id, 10)
            .await
            .unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].status, "healthy");
    }

    #[tokio::test]
    async fn ssh_probe_flags_non_ssh_service_on_open_port() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
                let _ = socket.shutdown().await;
            }
        });

        let catalog = Catalog::in_memory().await.unwrap();
        let asset = catalog
            .add_asset(NewAsset::new(
                "wrong-port",
                addr.ip().to_string(),
                i64::from(addr.port()),
            ))
            .await
            .unwrap();

        let probed = probe_asset(&catalog, None, &asset, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(probed.health.status, "failed");
        assert_eq!(
            probed.health.error_code.as_deref(),
            Some("ssh_banner_invalid")
        );
    }

    #[tokio::test]
    async fn tcp_probe_succeeds_on_plain_tcp_listener_and_fails_on_closed_port() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        let closed_probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let closed_addr = closed_probe.local_addr().unwrap();
        drop(closed_probe);

        let catalog = Catalog::in_memory().await.unwrap();
        let open_asset = catalog
            .add_asset(NewAsset {
                name: "db-open".to_string(),
                protocol: ASSET_PROTOCOL_TCP.to_string(),
                hostname: open_addr.ip().to_string(),
                port: i64::from(open_addr.port()),
                display_name: None,
                description: None,
                tags: Vec::new(),
                credential_id: None,
                via_asset_id: None,
                allow_tcp_forwarding: true,
            })
            .await
            .unwrap();
        let closed_asset = catalog
            .add_asset(NewAsset {
                name: "db-closed".to_string(),
                protocol: ASSET_PROTOCOL_TCP.to_string(),
                hostname: closed_addr.ip().to_string(),
                port: i64::from(closed_addr.port()),
                display_name: None,
                description: None,
                tags: Vec::new(),
                credential_id: None,
                via_asset_id: None,
                allow_tcp_forwarding: true,
            })
            .await
            .unwrap();

        let results = probe_all_assets(&catalog, None, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        let closed = results.iter().find(|a| a.id == closed_asset.id).unwrap();
        let open = results.iter().find(|a| a.id == open_asset.id).unwrap();
        assert_eq!(open.health.status, "healthy");
        assert_eq!(closed.health.status, "failed");
        assert_eq!(
            closed.health.error_code.as_deref(),
            Some("tcp_connect_failed")
        );
    }
}
