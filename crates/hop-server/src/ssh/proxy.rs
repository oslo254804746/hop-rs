use std::{
    fmt, io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use anyhow::{Context, Result};
use hop_core::{validate_tcp_port, Asset, HopDb, NewSession};
use russh::{server, Channel};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
};

use super::{
    server::{audit_denied_attempt, AuthInfo, AUTHORIZATION_DENIED},
    session_registry::{ActiveSessionRegistry, TERMINATED_BY_ADMIN},
};

#[derive(Debug, Clone, Copy)]
enum RelayPeer {
    Client,
    Target,
}

impl fmt::Display for RelayPeer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Client => f.write_str("client"),
            Self::Target => f.write_str("target"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum RelayOperation {
    Read,
    Write,
}

impl fmt::Display for RelayOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read => f.write_str("read"),
            Self::Write => f.write_str("write"),
        }
    }
}

#[derive(Debug)]
struct RelayIoError {
    peer: RelayPeer,
    operation: RelayOperation,
    bytes: u64,
    source: io::Error,
}

impl RelayIoError {
    fn new(peer: RelayPeer, operation: RelayOperation, bytes: u64, source: io::Error) -> Self {
        Self {
            peer,
            operation,
            bytes,
            source,
        }
    }
}

impl fmt::Display for RelayIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} failed after {} bytes: {}",
            self.peer, self.operation, self.bytes, self.source
        )
    }
}

impl std::error::Error for RelayIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

fn relay_error_message(
    err: &RelayIoError,
    client_to_target_bytes: u64,
    target_to_client_bytes: u64,
) -> String {
    format!(
        "{err}; totals client->target={client_to_target_bytes} bytes, target->client={target_to_client_bytes} bytes"
    )
}

async fn relay_copy<R, W>(
    reader: &mut R,
    writer: &mut W,
    read_peer: RelayPeer,
    write_peer: RelayPeer,
    total_written: Arc<AtomicU64>,
) -> std::result::Result<u64, RelayIoError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = [0_u8; 16 * 1024];
    let mut written = 0_u64;
    loop {
        let read = reader
            .read(&mut buf)
            .await
            .map_err(|err| RelayIoError::new(read_peer, RelayOperation::Read, written, err))?;
        if read == 0 {
            return Ok(written);
        }
        writer
            .write_all(&buf[..read])
            .await
            .map_err(|err| RelayIoError::new(write_peer, RelayOperation::Write, written, err))?;
        written += read as u64;
        total_written.store(written, Ordering::Relaxed);
    }
}

pub async fn bridge_direct_tcpip(
    channel: Channel<server::Msg>,
    db: HopDb,
    active_sessions: ActiveSessionRegistry,
    auth: AuthInfo,
    asset: Asset,
    client_ip: Option<String>,
) -> Result<()> {
    if !db.key_can_access_asset(&auth.key_id, &asset.id).await? {
        audit_denied_attempt(
            &db,
            &auth,
            "tcp-forward",
            Some(asset.name.clone()),
            Some(asset.hostname.clone()),
            Some(asset.port),
            client_ip,
        )
        .await;
        anyhow::bail!(AUTHORIZATION_DENIED);
    }
    let session = db
        .start_session(NewSession {
            key_finger: auth.fingerprint.clone(),
            key_name: Some(auth.name.clone()),
            mode: "tcp-forward".to_string(),
            asset_name: Some(asset.name.clone()),
            target_host: Some(asset.hostname.clone()),
            target_port: Some(asset.port),
            client_ip,
        })
        .await?;

    let (terminate_tx, mut terminate_rx) = mpsc::unbounded_channel();
    active_sessions
        .register(session.id.clone(), terminate_tx)
        .await;
    let mut terminated = false;
    let result = async {
        let port = validate_tcp_port(asset.port)?;
        let stream = tokio::select! {
            Some(()) = terminate_rx.recv() => {
                terminated = true;
                let _ = channel.close().await;
                return Ok(());
            }
            stream = TcpStream::connect((asset.hostname.as_str(), port)) => {
                stream.with_context(|| format!("connect {}:{}", asset.hostname, asset.port))?
            }
        };
        let (mut tcp_read, mut tcp_write) = stream.into_split();
        let (mut channel_read, channel_write) = channel.split();
        let mut channel_reader = channel_read.make_reader();
        let mut channel_writer = channel_write.make_writer();
        let client_to_target_bytes = Arc::new(AtomicU64::new(0));
        let target_to_client_bytes = Arc::new(AtomicU64::new(0));

        tokio::select! {
            Some(()) = terminate_rx.recv() => {
                terminated = true;
                let _ = tcp_write.shutdown().await;
                let _ = channel_write.eof().await;
                let _ = channel_write.close().await;
            }
            res = relay_copy(
                &mut channel_reader,
                &mut tcp_write,
                RelayPeer::Client,
                RelayPeer::Target,
                client_to_target_bytes.clone(),
            ) => {
                if let Err(err) = res {
                    anyhow::bail!(
                        "{}",
                        relay_error_message(
                            &err,
                            client_to_target_bytes.load(Ordering::Relaxed),
                            target_to_client_bytes.load(Ordering::Relaxed),
                        )
                    );
                }
                let _ = tcp_write.shutdown().await;
            }
            res = relay_copy(
                &mut tcp_read,
                &mut channel_writer,
                RelayPeer::Target,
                RelayPeer::Client,
                target_to_client_bytes.clone(),
            ) => {
                if let Err(err) = res {
                    anyhow::bail!(
                        "{}",
                        relay_error_message(
                            &err,
                            client_to_target_bytes.load(Ordering::Relaxed),
                            target_to_client_bytes.load(Ordering::Relaxed),
                        )
                    );
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    active_sessions.unregister(&session.id).await;
    match &result {
        _ if terminated => {
            db.finish_session(&session.id, "terminated", Some(TERMINATED_BY_ADMIN))
                .await?
        }
        Ok(_) => db.finish_session(&session.id, "ok", None).await?,
        Err(err) => {
            db.finish_session(&session.id, "failed", Some(&err.to_string()))
                .await?
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc,
        },
    };

    use super::*;

    #[test]
    fn relay_error_message_names_failing_side_and_byte_totals() {
        let err = RelayIoError::new(
            RelayPeer::Target,
            RelayOperation::Read,
            0,
            io::Error::new(io::ErrorKind::ConnectionReset, "reset by peer"),
        );

        let message = relay_error_message(&err, 19, 0);

        assert_eq!(
            message,
            "target read failed after 0 bytes: reset by peer; totals client->target=19 bytes, target->client=0 bytes"
        );
    }

    #[tokio::test]
    async fn relay_copy_counts_bytes_written() {
        let mut input = &b"rdp negotiation bytes"[..];
        let mut output = tokio::io::sink();
        let count = Arc::new(AtomicU64::new(0));

        let copied = relay_copy(
            &mut input,
            &mut output,
            RelayPeer::Client,
            RelayPeer::Target,
            count.clone(),
        )
        .await
        .unwrap();

        assert_eq!(copied, 21);
        assert_eq!(count.load(Ordering::Relaxed), 21);
    }
}
