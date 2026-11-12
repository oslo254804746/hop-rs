use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    sync::{Arc, OnceLock},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use hmac::{Hmac, Mac};
use hop_core::{
    decrypt_envelope, encrypt_envelope, notification_channel_crypto_context, Asset, HopCoreError,
    MasterKey, NotificationChannel, NotificationChannelKind,
};
use http::Uri;
use rustls::{
    pki_types::{pem::PemObject, CertificateDer, ServerName},
    ClientConfig, ClientConnection, RootCertStore, StreamOwned,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroize;

type HmacSha256 = Hmac<Sha256>;

const DEFAULT_TELEGRAM_API_BASE: &str = "https://api.telegram.org";
const DEFAULT_BARK_SERVER_BASE: &str = "https://api.day.app";
const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BODY_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TelegramConfigPayload {
    pub bot_token: String,
    pub chat_id: String,
    #[serde(default)]
    pub api_base_url: Option<String>,
}

impl Drop for TelegramConfigPayload {
    fn drop(&mut self) {
        self.bot_token.zeroize();
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BarkConfigPayload {
    #[serde(default)]
    pub server_url: Option<String>,
    pub device_key: String,
}

impl Drop for BarkConfigPayload {
    fn drop(&mut self) {
        self.device_key.zeroize();
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebhookConfigPayload {
    pub url: String,
    #[serde(default)]
    pub secret: Option<String>,
}

impl Drop for WebhookConfigPayload {
    fn drop(&mut self) {
        if let Some(secret) = &mut self.secret {
            secret.zeroize();
        }
    }
}

#[derive(Debug, Clone)]
pub enum DecryptedNotificationConfig {
    Telegram(TelegramConfigPayload),
    Bark(BarkConfigPayload),
    Webhook(WebhookConfigPayload),
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct NotificationConfigSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub has_secret: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationConfigInput {
    #[serde(default)]
    pub bot_token: Option<String>,
    #[serde(default)]
    pub chat_id: Option<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub device_key: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
}

impl Drop for NotificationConfigInput {
    fn drop(&mut self) {
        if let Some(token) = &mut self.bot_token {
            token.zeroize();
        }
        if let Some(key) = &mut self.device_key {
            key.zeroize();
        }
        if let Some(secret) = &mut self.secret {
            secret.zeroize();
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AlertEvent {
    AssetDown {
        asset_id: String,
        asset_name: String,
        protocol: String,
        hostname: String,
        port: i64,
        consecutive_failures: i64,
        error_code: Option<String>,
        error_message: Option<String>,
    },
    AssetRecovered {
        asset_id: String,
        asset_name: String,
        protocol: String,
        hostname: String,
        port: i64,
        latency_ms: Option<i64>,
    },
    ChannelTest {
        channel_name: String,
        channel_kind: String,
    },
}

impl AlertEvent {
    pub fn from_down_asset(asset: &Asset) -> Self {
        Self::AssetDown {
            asset_id: asset.id.clone(),
            asset_name: asset.name.clone(),
            protocol: asset.protocol.clone(),
            hostname: asset.hostname.clone(),
            port: asset.port,
            consecutive_failures: asset.health.consecutive_failures,
            error_code: asset.health.error_code.clone(),
            error_message: asset.health.error_message.clone(),
        }
    }

    pub fn from_recovered_asset(asset: &Asset) -> Self {
        Self::AssetRecovered {
            asset_id: asset.id.clone(),
            asset_name: asset.name.clone(),
            protocol: asset.protocol.clone(),
            hostname: asset.hostname.clone(),
            port: asset.port,
            latency_ms: asset.health.latency_ms,
        }
    }

    pub fn format_title_and_body(&self) -> (String, String) {
        match self {
            Self::AssetDown {
                asset_name,
                protocol,
                hostname,
                port,
                consecutive_failures,
                error_code,
                error_message,
                ..
            } => {
                let title = format!("[Hop 告警] {asset_name} 失联");
                let diagnosis = describe_probe_error(error_code.as_deref());
                let mut body = format!(
                    "资产: {asset_name}\n地址: {hostname}:{port} ({})\n连续失败: {consecutive_failures} 次\n诊断: {diagnosis}",
                    protocol.to_ascii_uppercase()
                );
                if let Some(detail) = error_message.as_deref().filter(|s| !s.trim().is_empty()) {
                    let short: String = detail.chars().take(180).collect();
                    body.push_str(&format!("\n详情: {short}"));
                }
                (title, body)
            }
            Self::AssetRecovered {
                asset_name,
                protocol,
                hostname,
                port,
                latency_ms,
                ..
            } => {
                let title = format!("[Hop 恢复] {asset_name} 已恢复在线");
                let latency_str = latency_ms
                    .map(|ms| format!("{ms} ms"))
                    .unwrap_or_else(|| "ok".to_string());
                let body = format!(
                    "资产: {asset_name}\n地址: {hostname}:{port} ({})\n当前延迟: {latency_str}",
                    protocol.to_ascii_uppercase()
                );
                (title, body)
            }
            Self::ChannelTest {
                channel_name,
                channel_kind,
            } => (
                "[Hop 测试] 通知通道验证成功".to_string(),
                format!(
                    "通道名称: {channel_name}\n通道类型: {channel_kind}\nHop 拨测与恢复通知已就绪。"
                ),
            ),
        }
    }
}

fn describe_probe_error(code: Option<&str>) -> &'static str {
    match code {
        Some("tcp_connect_failed") => "TCP 连接失败（端口未开放、路由不可达或 IP 阻断）",
        Some("probe_timeout") => "TCP 连接超时（链路丢包或防火墙丢弃 SYN）",
        Some("ssh_banner_timeout") => {
            "TCP 已连通但 SSH Banner 超时（疑似防火墙协议限流/阻断或 sshd 假死）"
        }
        Some("ssh_banner_eof") => "SSH 握手前连接被对端关闭（疑似 Fail2Ban / TCP Wrapper 拦截）",
        Some("ssh_banner_invalid") => "目标端口已开放但响应非 SSH 协议",
        Some("via_jump_failed") => "前置跳板机（via_asset）隧道建立失败",
        Some(other) if !other.is_empty() => "拨测异常",
        _ => "连接检测失败",
    }
}

pub fn decrypt_notification_config(
    master_key: &MasterKey,
    channel: &NotificationChannel,
) -> Result<DecryptedNotificationConfig> {
    let kind = NotificationChannelKind::try_from(channel.kind.as_str())?;
    let context = notification_channel_crypto_context(&channel.id);
    let mut raw = decrypt_envelope(master_key, &context, &channel.config_enc)?;
    let parsed = match kind {
        NotificationChannelKind::Telegram => serde_json::from_slice::<TelegramConfigPayload>(&raw)
            .map(DecryptedNotificationConfig::Telegram)
            .context("parse telegram notification config"),
        NotificationChannelKind::Bark => serde_json::from_slice::<BarkConfigPayload>(&raw)
            .map(DecryptedNotificationConfig::Bark)
            .context("parse bark notification config"),
        NotificationChannelKind::Webhook => serde_json::from_slice::<WebhookConfigPayload>(&raw)
            .map(DecryptedNotificationConfig::Webhook)
            .context("parse webhook notification config"),
    };
    raw.zeroize();
    parsed
}

pub fn summarize_notification_config(
    master_key: &MasterKey,
    channel: &NotificationChannel,
) -> NotificationConfigSummary {
    match decrypt_notification_config(master_key, channel) {
        Ok(DecryptedNotificationConfig::Telegram(cfg)) => NotificationConfigSummary {
            chat_id: Some(cfg.chat_id.clone()),
            api_base_url: cfg.api_base_url.clone(),
            has_secret: !cfg.bot_token.trim().is_empty(),
            ..NotificationConfigSummary::default()
        },
        Ok(DecryptedNotificationConfig::Bark(cfg)) => NotificationConfigSummary {
            server_url: cfg.server_url.clone(),
            has_secret: !cfg.device_key.trim().is_empty(),
            ..NotificationConfigSummary::default()
        },
        Ok(DecryptedNotificationConfig::Webhook(cfg)) => NotificationConfigSummary {
            url: Some(cfg.url.clone()),
            has_secret: cfg
                .secret
                .as_deref()
                .is_some_and(|secret| !secret.trim().is_empty()),
            ..NotificationConfigSummary::default()
        },
        Err(_) => NotificationConfigSummary::default(),
    }
}

pub fn build_encrypted_notification_config(
    master_key: &MasterKey,
    id: &str,
    kind: NotificationChannelKind,
    input: &NotificationConfigInput,
    existing: Option<&DecryptedNotificationConfig>,
) -> Result<String, HopCoreError> {
    let payload_value = match kind {
        NotificationChannelKind::Telegram => {
            let prev = match existing {
                Some(DecryptedNotificationConfig::Telegram(cfg)) => Some(cfg),
                _ => None,
            };
            let bot_token = non_empty_trimmed(input.bot_token.as_deref())
                .or_else(|| prev.map(|cfg| cfg.bot_token.as_str()))
                .ok_or_else(|| {
                    HopCoreError::Validation("telegram channel requires bot_token".to_string())
                })?;
            let chat_id = non_empty_trimmed(input.chat_id.as_deref())
                .or_else(|| prev.map(|cfg| cfg.chat_id.as_str()))
                .ok_or_else(|| {
                    HopCoreError::Validation("telegram channel requires chat_id".to_string())
                })?;
            let api_base_url = non_empty_trimmed(input.api_base_url.as_deref());
            if let Some(base) = api_base_url {
                validate_url(base, "api_base_url")?;
            }
            serde_json::json!({
                "bot_token": bot_token,
                "chat_id": chat_id,
                "api_base_url": api_base_url,
            })
        }
        NotificationChannelKind::Bark => {
            let prev = match existing {
                Some(DecryptedNotificationConfig::Bark(cfg)) => Some(cfg),
                _ => None,
            };
            let device_key = non_empty_trimmed(input.device_key.as_deref())
                .or_else(|| prev.map(|cfg| cfg.device_key.as_str()))
                .ok_or_else(|| {
                    HopCoreError::Validation("bark channel requires device_key".to_string())
                })?;
            let server_url = non_empty_trimmed(input.server_url.as_deref());
            if let Some(base) = server_url {
                validate_url(base, "server_url")?;
            }
            serde_json::json!({
                "server_url": server_url,
                "device_key": device_key,
            })
        }
        NotificationChannelKind::Webhook => {
            let prev = match existing {
                Some(DecryptedNotificationConfig::Webhook(cfg)) => Some(cfg),
                _ => None,
            };
            let url = non_empty_trimmed(input.url.as_deref())
                .or_else(|| prev.map(|cfg| cfg.url.as_str()))
                .ok_or_else(|| {
                    HopCoreError::Validation("webhook channel requires url".to_string())
                })?;
            validate_url(url, "url")?;
            let secret = non_empty_trimmed(input.secret.as_deref())
                .or_else(|| prev.and_then(|cfg| cfg.secret.as_deref()));
            serde_json::json!({
                "url": url,
                "secret": secret,
            })
        }
    };

    let mut bytes = serde_json::to_vec(&payload_value)
        .map_err(|err| HopCoreError::Validation(format!("serialize notification config: {err}")))?;
    let context = notification_channel_crypto_context(id);
    let encrypted = encrypt_envelope(master_key, &context, &bytes);
    bytes.zeroize();
    encrypted
}

fn non_empty_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn validate_url(raw: &str, field: &str) -> Result<(), HopCoreError> {
    let uri = raw.parse::<Uri>().map_err(|_| {
        HopCoreError::Validation(format!("{field} must be a valid http:// or https:// URL"))
    })?;
    if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.authority().is_none() {
        return Err(HopCoreError::Validation(format!(
            "{field} must start with http:// or https:// and include a host"
        )));
    }
    Ok(())
}

pub async fn send_channel_notification(
    master_key: &MasterKey,
    channel: &NotificationChannel,
    event: &AlertEvent,
) -> Result<()> {
    let decrypted = decrypt_notification_config(master_key, channel)?;
    let (title, body) = event.format_title_and_body();

    match decrypted {
        DecryptedNotificationConfig::Telegram(cfg) => {
            let base = cfg
                .api_base_url
                .as_deref()
                .unwrap_or(DEFAULT_TELEGRAM_API_BASE)
                .trim_end_matches('/');
            let endpoint = format!("{base}/bot{}/sendMessage", cfg.bot_token.trim());
            let payload = serde_json::to_vec(&serde_json::json!({
                "chat_id": cfg.chat_id.trim(),
                "text": format!("{title}\n\n{body}"),
                "disable_web_page_preview": true,
            }))?;
            post_json(&endpoint, payload, Vec::new(), DEFAULT_HTTP_TIMEOUT).await
        }
        DecryptedNotificationConfig::Bark(cfg) => {
            let base = cfg
                .server_url
                .as_deref()
                .unwrap_or(DEFAULT_BARK_SERVER_BASE)
                .trim_end_matches('/');
            let endpoint = format!("{base}/push");
            let payload = serde_json::to_vec(&serde_json::json!({
                "device_key": cfg.device_key.trim(),
                "title": title,
                "body": body,
                "group": "Hop",
            }))?;
            post_json(&endpoint, payload, Vec::new(), DEFAULT_HTTP_TIMEOUT).await
        }
        DecryptedNotificationConfig::Webhook(cfg) => {
            let payload = serde_json::to_vec(&serde_json::json!({
                "title": title,
                "message": body,
                "channel": channel.name,
                "payload": event,
            }))?;
            let mut extra_headers = Vec::new();
            if let Some(secret) = cfg.secret.as_deref().filter(|s| !s.trim().is_empty()) {
                let sig = compute_hmac_sha256_hex(secret.trim().as_bytes(), &payload)?;
                extra_headers.push(("X-Hop-Signature-256".to_string(), format!("sha256={sig}")));
            }
            post_json(cfg.url.trim(), payload, extra_headers, DEFAULT_HTTP_TIMEOUT).await
        }
    }
}

fn compute_hmac_sha256_hex(secret: &[u8], body: &[u8]) -> Result<String> {
    let mut mac = HmacSha256::new_from_slice(secret).context("initialize webhook HMAC key")?;
    mac.update(body);
    let digest = mac.finalize().into_bytes();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    Ok(out)
}

async fn post_json(
    url: &str,
    body: Vec<u8>,
    extra_headers: Vec<(String, String)>,
    timeout: Duration,
) -> Result<()> {
    let target_url = url.to_string();
    tokio::task::spawn_blocking(move || {
        post_json_blocking(&target_url, &body, &extra_headers, timeout)
    })
    .await
    .context("notification HTTP worker panicked")?
}

fn post_json_blocking(
    url: &str,
    body: &[u8],
    extra_headers: &[(String, String)],
    timeout: Duration,
) -> Result<()> {
    let uri: Uri = url.parse().context("invalid notification endpoint URL")?;
    let scheme = uri.scheme_str().unwrap_or("https");
    let is_https = match scheme {
        "https" => true,
        "http" => false,
        other => bail!("unsupported notification URL scheme: {other}"),
    };
    let host = uri
        .host()
        .context("notification URL missing host")?
        .trim_matches(['[', ']']);
    let port = uri.port_u16().unwrap_or(if is_https { 443 } else { 80 });
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let host_header = match (is_https, port) {
        (true, 443) | (false, 80) => host.to_string(),
        _ => format!("{host}:{port}"),
    };

    let proxy_uri = resolve_proxy_for_host(host, is_https);

    let (status_code, response_snippet) = if is_https {
        let tcp = if let Some(proxy) = proxy_uri {
            connect_via_http_proxy(&proxy, host, port, timeout)?
        } else {
            connect_direct_tcp(host, port, timeout)?
        };
        let tls_config = shared_rustls_config()?;
        let server_name = ServerName::try_from(host.to_string())
            .map_err(|_| anyhow::anyhow!("invalid TLS server name '{host}'"))?;
        let conn = ClientConnection::new(tls_config, server_name)
            .context("initialize TLS client connection")?;
        let mut tls_stream = StreamOwned::new(conn, tcp);
        exchange_http_post(
            &mut tls_stream,
            path_and_query,
            &host_header,
            body,
            extra_headers,
        )?
    } else if let Some(proxy) = proxy_uri {
        let proxy_host = proxy
            .host()
            .context("HTTP_PROXY missing host")?
            .trim_matches(['[', ']']);
        let proxy_port = proxy.port_u16().unwrap_or(8080);
        let mut tcp = connect_direct_tcp(proxy_host, proxy_port, timeout)?;
        exchange_http_post(&mut tcp, url, &host_header, body, extra_headers)?
    } else {
        let mut tcp = connect_direct_tcp(host, port, timeout)?;
        exchange_http_post(&mut tcp, path_and_query, &host_header, body, extra_headers)?
    };

    if (200..300).contains(&status_code) {
        Ok(())
    } else {
        bail!(
            "notification endpoint returned HTTP {}: {}",
            status_code,
            response_snippet
        )
    }
}

fn connect_direct_tcp(host: &str, port: u16, timeout: Duration) -> Result<TcpStream> {
    let mut addrs = (host, port)
        .to_socket_addrs()
        .with_context(|| format!("resolve notification host {host}:{port}"))?;
    let first: SocketAddr = addrs
        .next()
        .with_context(|| format!("no socket address resolved for {host}:{port}"))?;
    let stream = TcpStream::connect_timeout(&first, timeout)
        .with_context(|| format!("connect notification endpoint {first}"))?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    Ok(stream)
}

fn connect_via_http_proxy(
    proxy: &Uri,
    target_host: &str,
    target_port: u16,
    timeout: Duration,
) -> Result<TcpStream> {
    let proxy_host = proxy
        .host()
        .context("HTTPS_PROXY missing host")?
        .trim_matches(['[', ']']);
    let proxy_port = proxy.port_u16().unwrap_or(8080);
    let mut stream = connect_direct_tcp(proxy_host, proxy_port, timeout)?;
    let connect_req = format!(
        "CONNECT {target_host}:{target_port} HTTP/1.1\r\nHost: {target_host}:{target_port}\r\nProxy-Connection: Keep-Alive\r\n\r\n"
    );
    stream
        .write_all(connect_req.as_bytes())
        .context("write HTTP CONNECT to proxy")?;
    stream.flush().context("flush HTTP CONNECT to proxy")?;

    let mut header_bytes = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while header_bytes.len() < 8192 {
        let n = stream
            .read(&mut byte)
            .context("read HTTP CONNECT response from proxy")?;
        if n == 0 {
            break;
        }
        header_bytes.push(byte[0]);
        if header_bytes.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let header_text = String::from_utf8_lossy(&header_bytes);
    let first_line = header_text.lines().next().unwrap_or_default();
    let status = first_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    if !(200..300).contains(&status) {
        bail!("proxy CONNECT to {target_host}:{target_port} failed: {first_line}");
    }
    Ok(stream)
}

fn exchange_http_post<S: Read + Write>(
    stream: &mut S,
    request_target: &str,
    host_header: &str,
    body: &[u8],
    extra_headers: &[(String, String)],
) -> Result<(u16, String)> {
    let mut req = format!(
        "POST {request_target} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: hop-server/{}\r\nContent-Type: application/json; charset=utf-8\r\nAccept: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        env!("CARGO_PKG_VERSION"),
        body.len()
    );
    for (name, val) in extra_headers {
        if !name.contains(['\r', '\n']) && !val.contains(['\r', '\n']) {
            req.push_str(&format!("{name}: {val}\r\n"));
        }
    }
    req.push_str("\r\n");

    stream
        .write_all(req.as_bytes())
        .context("send HTTP request headers")?;
    stream
        .write_all(body)
        .context("send HTTP request JSON payload")?;
    stream.flush().context("flush HTTP request")?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .context("read HTTP status line")?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .with_context(|| format!("unexpected HTTP status line: {}", status_line.trim()))?;

    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).context("read HTTP header")?;
        if read == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }

    let mut resp_body = Vec::new();
    let _ = reader
        .take(MAX_RESPONSE_BODY_BYTES as u64)
        .read_to_end(&mut resp_body);
    let snippet = String::from_utf8_lossy(&resp_body)
        .trim()
        .chars()
        .take(240)
        .collect::<String>();
    Ok((status_code, snippet))
}

fn resolve_proxy_for_host(host: &str, is_https: bool) -> Option<Uri> {
    if matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return None;
    }
    if let Ok(no_proxy) = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")) {
        for pat in no_proxy.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if pat == "*" || host.eq_ignore_ascii_case(pat) || host.ends_with(pat) {
                return None;
            }
        }
    }
    let env_val = if is_https {
        std::env::var("HTTPS_PROXY")
            .or_else(|_| std::env::var("https_proxy"))
            .ok()
    } else {
        std::env::var("HTTP_PROXY")
            .or_else(|_| std::env::var("http_proxy"))
            .ok()
    }?;
    let trimmed = env_val.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<Uri>().ok()
}

fn shared_rustls_config() -> Result<Arc<ClientConfig>> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    if let Some(existing) = CONFIG.get() {
        return Ok(Arc::clone(existing));
    }

    let mut root_store = RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let extra_ca_paths = [
        std::env::var("SSL_CERT_FILE").ok(),
        Some("/etc/ssl/certs/ca-certificates.crt".to_string()),
    ];
    for candidate in extra_ca_paths.into_iter().flatten() {
        if let Ok(pem) = std::fs::read(&candidate) {
            for cert in CertificateDer::pem_slice_iter(&pem).flatten() {
                let _ = root_store.add(cert);
            }
        }
    }

    let built =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .context("configure TLS protocol versions")?
            .with_root_certificates(root_store)
            .with_no_client_auth();

    let arc = Arc::new(built);
    let _ = CONFIG.set(Arc::clone(&arc));
    Ok(arc)
}

#[cfg(test)]
pub(crate) mod tests {
    use hop_core::{MasterKey, NotificationChannel, NotificationChannelKind};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream as TokioTcpStream},
    };

    use super::*;

    pub(crate) async fn read_http_request_for_test(socket: &mut TokioTcpStream) -> String {
        let mut buf = Vec::with_capacity(4096);
        let mut chunk = [0u8; 1024];
        loop {
            let n = socket.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(header_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let body_start = header_end + 4;
                let headers_str = String::from_utf8_lossy(&buf[..header_end]);
                let content_len = headers_str
                    .lines()
                    .find_map(|line| {
                        let (k, v) = line.split_once(':')?;
                        if k.trim().eq_ignore_ascii_case("content-length") {
                            v.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                while buf.len() < body_start + content_len {
                    let m = socket.read(&mut chunk).await.unwrap_or(0);
                    if m == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..m]);
                }
                break;
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    async fn spawn_single_http_capture() -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let raw = read_http_request_for_test(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}")
                .await
                .unwrap();
            let _ = socket.shutdown().await;
            raw
        });
        (format!("http://127.0.0.1:{}", addr.port()), handle)
    }

    #[tokio::test]
    async fn telegram_bark_and_webhook_channels_deliver_formatted_json_and_hmac() {
        let master_key = MasterKey::from_bytes([31; 32]);

        // 1. Telegram
        let (tg_base, tg_capture) = spawn_single_http_capture().await;
        let tg_id = "chan-tg";
        let mut tg_input = NotificationConfigInput::default();
        tg_input.bot_token = Some("123456:ABCDEF".to_string());
        tg_input.chat_id = Some("-100987654".to_string());
        tg_input.api_base_url = Some(tg_base);
        let tg_enc = build_encrypted_notification_config(
            &master_key,
            tg_id,
            NotificationChannelKind::Telegram,
            &tg_input,
            None,
        )
        .unwrap();
        let tg_channel = NotificationChannel {
            id: tg_id.to_string(),
            name: "ops-tg".to_string(),
            kind: "telegram".to_string(),
            enabled: true,
            config_enc: tg_enc,
            created_at: None,
            updated_at: None,
        };
        let summary = summarize_notification_config(&master_key, &tg_channel);
        assert_eq!(summary.chat_id.as_deref(), Some("-100987654"));
        assert!(summary.has_secret);

        send_channel_notification(
            &master_key,
            &tg_channel,
            &AlertEvent::AssetDown {
                asset_id: "a1".to_string(),
                asset_name: "hk-cn2".to_string(),
                protocol: "ssh".to_string(),
                hostname: "203.0.113.8".to_string(),
                port: 22,
                consecutive_failures: 2,
                error_code: Some("ssh_banner_timeout".to_string()),
                error_message: Some("banner timed out".to_string()),
            },
        )
        .await
        .unwrap();
        let tg_req = tg_capture.await.unwrap();
        assert!(tg_req.starts_with("POST /bot123456:ABCDEF/sendMessage HTTP/1.1"));
        assert!(tg_req.contains("-100987654"));
        assert!(tg_req.contains("hk-cn2"));

        // 2. Bark
        let (bark_base, bark_capture) = spawn_single_http_capture().await;
        let bark_id = "chan-bark";
        let mut bark_input = NotificationConfigInput::default();
        bark_input.server_url = Some(bark_base);
        bark_input.device_key = Some("bark-key-xyz".to_string());
        let bark_enc = build_encrypted_notification_config(
            &master_key,
            bark_id,
            NotificationChannelKind::Bark,
            &bark_input,
            None,
        )
        .unwrap();
        let bark_channel = NotificationChannel {
            id: bark_id.to_string(),
            name: "iphone-bark".to_string(),
            kind: "bark".to_string(),
            enabled: true,
            config_enc: bark_enc,
            created_at: None,
            updated_at: None,
        };
        send_channel_notification(
            &master_key,
            &bark_channel,
            &AlertEvent::ChannelTest {
                channel_name: "iphone-bark".to_string(),
                channel_kind: "bark".to_string(),
            },
        )
        .await
        .unwrap();
        let bark_req = bark_capture.await.unwrap();
        assert!(bark_req.starts_with("POST /push HTTP/1.1"));
        assert!(bark_req.contains("bark-key-xyz"));

        // 3. Webhook with HMAC-SHA256
        let (wh_base, wh_capture) = spawn_single_http_capture().await;
        let wh_id = "chan-wh";
        let mut wh_input = NotificationConfigInput::default();
        wh_input.url = Some(format!("{wh_base}/hooks/hop"));
        wh_input.secret = Some("topsecret".to_string());
        let wh_enc = build_encrypted_notification_config(
            &master_key,
            wh_id,
            NotificationChannelKind::Webhook,
            &wh_input,
            None,
        )
        .unwrap();
        let wh_channel = NotificationChannel {
            id: wh_id.to_string(),
            name: "custom-hook".to_string(),
            kind: "webhook".to_string(),
            enabled: true,
            config_enc: wh_enc,
            created_at: None,
            updated_at: None,
        };
        send_channel_notification(
            &master_key,
            &wh_channel,
            &AlertEvent::AssetRecovered {
                asset_id: "a1".to_string(),
                asset_name: "hk-cn2".to_string(),
                protocol: "ssh".to_string(),
                hostname: "203.0.113.8".to_string(),
                port: 22,
                latency_ms: Some(36),
            },
        )
        .await
        .unwrap();
        let wh_req = wh_capture.await.unwrap();
        assert!(wh_req.starts_with("POST /hooks/hop HTTP/1.1"));
        assert!(wh_req.contains("X-Hop-Signature-256: sha256="));
    }
}
