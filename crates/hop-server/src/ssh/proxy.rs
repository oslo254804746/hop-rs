use anyhow::{Context, Result};
use hop_core::{Asset, HopDb, NewSession};
use russh::{server, Channel};
use tokio::{io::AsyncWriteExt, net::TcpStream};

use super::server::AuthInfo;

pub async fn bridge_direct_tcpip(channel: Channel<server::Msg>, db: HopDb, auth: AuthInfo, asset: Asset, client_ip: Option<String>) -> Result<()> {
    let session = db
        .start_session(NewSession {
            key_finger: auth.fingerprint,
            key_name: Some(auth.name),
            mode: "proxyjump".to_string(),
            asset_name: Some(asset.name.clone()),
            target_host: Some(asset.hostname.clone()),
            target_port: Some(asset.port),
            client_ip,
        })
        .await?;

    let result = async {
        let stream = TcpStream::connect((asset.hostname.as_str(), asset.port as u16))
            .await
            .with_context(|| format!("connect {}:{}", asset.hostname, asset.port))?;
        let (mut tcp_read, mut tcp_write) = stream.into_split();
        let (mut channel_read, channel_write) = channel.split();
        let mut channel_reader = channel_read.make_reader();
        let mut channel_writer = channel_write.make_writer();

        tokio::select! {
            res = tokio::io::copy(&mut channel_reader, &mut tcp_write) => {
                res?;
                let _ = tcp_write.shutdown().await;
            }
            res = tokio::io::copy(&mut tcp_read, &mut channel_writer) => {
                res?;
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    match &result {
        Ok(_) => db.finish_session(&session.id, "ok", None).await?,
        Err(err) => db.finish_session(&session.id, "failed", Some(&err.to_string())).await?,
    }
    result
}
