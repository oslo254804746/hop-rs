use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use hop_core::{
    encrypt_envelope, new_id, Asset, AuthType, Catalog, Credential, HopCoreError, MasterKey,
    NewCredential, ResourceOwnership, ASSET_PROTOCOL_SSH,
};
use russh::{
    client,
    keys::{
        key::safe_rng,
        ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey},
    },
    ChannelMsg,
};
use zeroize::Zeroize;

use crate::ssh::client::{
    connect_asset_with_auth, wait_for_request_success, ClientAuthMaterial, ManagedClientSession,
};

const HARDEN_EXIT_NO_SUDO: u32 = 42;

#[derive(Debug, Clone)]
pub struct BootstrapKeyOptions {
    pub username: Option<String>,
    pub password: Option<String>,
    pub credential_name: Option<String>,
    pub disable_password_auth: bool,
    pub prune_previous_credential: bool,
}

impl Default for BootstrapKeyOptions {
    fn default() -> Self {
        Self {
            username: None,
            password: None,
            credential_name: None,
            disable_password_auth: false,
            prune_previous_credential: true,
        }
    }
}

impl Drop for BootstrapKeyOptions {
    fn drop(&mut self) {
        if let Some(password) = &mut self.password {
            password.zeroize();
        }
    }
}

#[derive(Debug, Clone)]
pub struct BootstrapKeyOutcome {
    pub asset: Asset,
    pub credential: Credential,
    pub public_key: String,
    pub public_key_fingerprint: String,
    pub verified_key_login: bool,
    pub pruned_previous_credential: bool,
    pub password_auth_disabled: bool,
    pub warnings: Vec<String>,
}

struct ExecOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_status: Option<u32>,
}

pub async fn bootstrap_asset_ssh_key(
    db: &Catalog,
    master_key: Arc<MasterKey>,
    asset_id: &str,
    options: BootstrapKeyOptions,
    timeout: Duration,
) -> Result<BootstrapKeyOutcome> {
    let asset = db
        .get_asset_by_id(asset_id)
        .await?
        .ok_or_else(|| HopCoreError::Validation("not_found: asset was not found".to_string()))?;

    if asset.protocol != ASSET_PROTOCOL_SSH {
        return Err(HopCoreError::Validation(
            "only ssh assets support SSH key bootstrap".to_string(),
        )
        .into());
    }

    let ownership = db.resource_ownership("asset", &asset.id).await?;
    if ownership != ResourceOwnership::Local {
        return Err(HopCoreError::Validation(format!(
            "managed_by_source: asset '{}' is managed by declarative source and cannot be mutated in-place",
            asset.name
        ))
        .into());
    }

    let existing_credential = match asset.credential_id.as_deref() {
        Some(cred_id) => db.get_credential(cred_id).await?,
        None => None,
    };

    let inline_password = options
        .password
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let requested_username = options
        .username
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    let (target_username, initial_auth) = match (inline_password, existing_credential.as_ref()) {
        (Some(password), existing) => {
            let username = requested_username
                .or_else(|| existing.map(|cred| cred.username.clone()))
                .ok_or_else(|| {
                    HopCoreError::Validation(
                        "username is required when bootstrapping an asset without an existing credential"
                            .to_string(),
                    )
                })?;
            (
                username.clone(),
                ClientAuthMaterial::InlinePassword { username, password },
            )
        }
        (None, Some(cred)) => {
            if let Some(explicit_user) = requested_username {
                if explicit_user != cred.username {
                    return Err(HopCoreError::Validation(format!(
                        "username '{}' does not match bound credential username '{}'; supply an inline password to switch users",
                        explicit_user, cred.username
                    ))
                    .into());
                }
            }
            (cred.username.clone(), ClientAuthMaterial::StoredCredential)
        }
        (None, None) => {
            return Err(HopCoreError::Validation(
                "asset has no bound credential; supply username and password to bootstrap"
                    .to_string(),
            )
            .into());
        }
    };

    let cred_id = new_id();
    let credential_name = options
        .credential_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            let short_id: String = cred_id.chars().take(8).collect();
            format!("managed-{}-{short_id}", sanitize_slug(&asset.name))
        });

    let mut generated_key = PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519)
        .context("generate Ed25519 SSH keypair")?;
    generated_key.set_comment(format!("hop-managed-{}", sanitize_slug(&asset.name)));

    let public_key = generated_key
        .public_key()
        .to_openssh()
        .context("encode OpenSSH public key")?;
    let public_key_fingerprint = format!(
        "{}",
        generated_key.public_key().fingerprint(HashAlg::Sha256)
    );
    let private_key_pem = generated_key
        .to_openssh(LineEnding::LF)
        .context("encode OpenSSH private key")?;

    let install_script = build_install_authorized_key_script(&public_key)?;
    let conn1 = connect_asset_with_auth(
        db.clone(),
        Arc::clone(&master_key),
        &asset,
        initial_auth,
        timeout,
    )
    .await
    .with_context(|| format!("initial SSH login to '{}' failed", asset.name))?;

    let install_res = run_remote_command(&conn1, install_script.as_bytes(), timeout).await;
    conn1.disconnect("hop bootstrap key installed").await;
    let install_out = install_res.context("failed executing authorized_keys install command")?;
    if install_out.exit_status != Some(0) {
        let stderr = summarize_output(&install_out.stderr, &install_out.stdout);
        bail!(
            "installing authorized_keys on '{}' failed (exit {:?}): {}",
            asset.name,
            install_out.exit_status,
            stderr
        );
    }

    let verify_started = Instant::now();
    let conn2 = connect_asset_with_auth(
        db.clone(),
        Arc::clone(&master_key),
        &asset,
        ClientAuthMaterial::InlinePrivateKey {
            username: target_username.clone(),
            private_key_pem: (*private_key_pem).clone(),
            passphrase: None,
        },
        timeout,
    )
    .await
    .with_context(|| {
        format!(
            "second-connection verification with newly installed Ed25519 key failed on '{}'; catalog credential was NOT modified",
            asset.name
        )
    })?;

    let verify_out = run_remote_command(&conn2, b"true", timeout)
        .await
        .context("failed running verification command over new SSH key session")?;
    if verify_out.exit_status != Some(0) {
        conn2.disconnect("hop bootstrap verify failed").await;
        bail!(
            "verification command over newly installed SSH key returned exit {:?}; catalog credential was NOT modified",
            verify_out.exit_status
        );
    }
    let verify_latency_ms = verify_started.elapsed().as_millis().min(i64::MAX as u128) as i64;

    let mut warnings = Vec::new();
    let mut password_auth_disabled = false;
    if options.disable_password_auth {
        let harden_script = build_disable_password_auth_script();
        match run_remote_command(&conn2, harden_script.as_bytes(), timeout).await {
            Ok(out) if out.exit_status == Some(0) => {
                password_auth_disabled = true;
            }
            Ok(out) if out.exit_status == Some(HARDEN_EXIT_NO_SUDO) => {
                warnings.push(
                    "SSH key verified and bound, but PasswordAuthentication could not be disabled automatically (requires root or passwordless sudo)."
                        .to_string(),
                );
            }
            Ok(out) => {
                let detail = summarize_output(&out.stderr, &out.stdout);
                warnings.push(format!(
                    "SSH key verified and bound, but sshd PasswordAuthentication hardening exited with {:?}: {detail}",
                    out.exit_status
                ));
            }
            Err(err) => {
                warnings.push(format!(
                    "SSH key verified and bound, but sshd PasswordAuthentication hardening failed: {err}"
                ));
            }
        }
    }
    conn2.disconnect("hop bootstrap complete").await;

    let private_key_enc = encrypt_envelope(
        &master_key,
        &format!("{cred_id}:private_key"),
        private_key_pem.as_bytes(),
    )?;

    let (updated_asset, created_credential, pruned_previous_credential) = db
        .bootstrap_bind_asset_credential(
            &asset.id,
            NewCredential {
                id: Some(cred_id),
                name: credential_name,
                username: target_username,
                auth_type: AuthType::Key,
                password_enc: None,
                private_key_enc: Some(private_key_enc),
                passphrase_enc: None,
            },
            options.prune_previous_credential,
        )
        .await?;

    let _ = db
        .record_asset_health_success(&updated_asset.id, Some(verify_latency_ms))
        .await;
    let _ = db
        .record_probe_sample(&updated_asset.id, true, Some(verify_latency_ms), None)
        .await;
    let refreshed_asset = db
        .get_asset_by_id(&updated_asset.id)
        .await?
        .unwrap_or(updated_asset);

    Ok(BootstrapKeyOutcome {
        asset: refreshed_asset,
        credential: created_credential,
        public_key,
        public_key_fingerprint,
        verified_key_login: true,
        pruned_previous_credential,
        password_auth_disabled,
        warnings,
    })
}

fn sanitize_slug(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars().take(48) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "asset".to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_install_authorized_key_script(public_key_line: &str) -> Result<String> {
    let line = public_key_line.trim();
    if line.is_empty()
        || line.contains('\'')
        || line.contains('\n')
        || line.contains('\r')
        || line.contains('\0')
    {
        bail!("generated OpenSSH public key line contains unexpected characters");
    }
    Ok(format!(
        "sh -c 'umask 077 && mkdir -p ~/.ssh && chmod 700 ~/.ssh && touch ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys && KEY=\"$1\" && if ! grep -qxF \"$KEY\" ~/.ssh/authorized_keys 2>/dev/null; then if [ -s ~/.ssh/authorized_keys ] && [ -n \"$(tail -c 1 ~/.ssh/authorized_keys 2>/dev/null || printf x)\" ]; then printf \"\\n\" >> ~/.ssh/authorized_keys; fi; printf \"%s\\n\" \"$KEY\" >> ~/.ssh/authorized_keys; fi' hop-bootstrap '{}'",
        line
    ))
}

fn build_disable_password_auth_script() -> String {
    format!(
        r##"sh -c '
set -e
if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
elif command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
  SUDO="sudo -n"
else
  exit {HARDEN_EXIT_NO_SUDO}
fi
SSHD_BIN="$(command -v sshd 2>/dev/null || echo /usr/sbin/sshd)"
if [ ! -x "$SSHD_BIN" ]; then
  echo "sshd binary not found" >&2
  exit 43
fi
if [ -d /etc/ssh/sshd_config.d ] && grep -Eq "^[[:space:]]*Include[[:space:]]+/etc/ssh/sshd_config\.d/" /etc/ssh/sshd_config 2>/dev/null; then
  CONF="/etc/ssh/sshd_config.d/99-hop-managed.conf"
  printf "# Managed by Hop SSH key bootstrap\nPasswordAuthentication no\nKbdInteractiveAuthentication no\n" | $SUDO tee "$CONF" >/dev/null
  if ! $SUDO "$SSHD_BIN" -t 2>/dev/null; then
    $SUDO rm -f "$CONF"
    echo "sshd -t rejected drop-in config" >&2
    exit 44
  fi
else
  BAK="/etc/ssh/sshd_config.hop.bak"
  $SUDO cp /etc/ssh/sshd_config "$BAK"
  if grep -Eq "^[[:space:]#]*PasswordAuthentication[[:space:]]+" /etc/ssh/sshd_config; then
    $SUDO sed -i "s/^[[:space:]#]*PasswordAuthentication[[:space:]].*/PasswordAuthentication no/" /etc/ssh/sshd_config
  else
    printf "\nPasswordAuthentication no\n" | $SUDO tee -a /etc/ssh/sshd_config >/dev/null
  fi
  if ! $SUDO "$SSHD_BIN" -t 2>/dev/null; then
    $SUDO mv "$BAK" /etc/ssh/sshd_config
    echo "sshd -t rejected updated sshd_config" >&2
    exit 44
  fi
  $SUDO rm -f "$BAK"
fi
$SUDO systemctl reload ssh 2>/dev/null || $SUDO systemctl reload sshd 2>/dev/null || $SUDO service ssh reload 2>/dev/null || $SUDO service sshd reload 2>/dev/null || $SUDO /etc/init.d/sshd reload 2>/dev/null || true
'"##
    )
}

async fn run_remote_command(
    client_session: &ManagedClientSession,
    command: &[u8],
    timeout: Duration,
) -> Result<ExecOutput> {
    let mut channel: russh::Channel<client::Msg> =
        tokio::time::timeout(timeout, client_session.session.channel_open_session())
            .await
            .context("opening SSH session channel timed out")?
            .context("open SSH session channel")?;

    channel
        .exec(true, command.to_vec())
        .await
        .context("send SSH exec request")?;
    wait_for_request_success(&mut channel, timeout, "exec").await?;
    let _ = channel.eof().await;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;

    tokio::time::timeout(timeout, async {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    if stdout.len() < 16 * 1024 {
                        stdout.extend_from_slice(&data);
                    }
                }
                Some(ChannelMsg::ExtendedData { data, ext: 1 }) => {
                    if stderr.len() < 16 * 1024 {
                        stderr.extend_from_slice(&data);
                    }
                }
                Some(ChannelMsg::ExitStatus {
                    exit_status: status,
                }) => {
                    exit_status = Some(status);
                }
                Some(ChannelMsg::Close) | None => break,
                _ => {}
            }
        }
    })
    .await
    .context("remote SSH command timed out waiting for completion")?;

    Ok(ExecOutput {
        stdout,
        stderr,
        exit_status,
    })
}

fn summarize_output(stderr: &[u8], stdout: &[u8]) -> String {
    let raw = if !stderr.is_empty() { stderr } else { stdout };
    let text = String::from_utf8_lossy(raw).trim().to_string();
    if text.is_empty() {
        "no output".to_string()
    } else {
        text.chars().take(300).collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{
        net::SocketAddr,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex as StdMutex,
        },
        time::Duration,
    };

    use hop_core::{encrypt_envelope, AuthType, Catalog, MasterKey, NewAsset, NewCredential};
    use russh::{
        keys::{key::safe_rng, ssh_key::PublicKey, Algorithm},
        server::{self, Auth, Msg as ServerMsg, Server as _, Session},
        Channel, ChannelId,
    };
    use tokio::net::TcpListener;

    use super::*;

    pub(crate) async fn spawn_test_bootstrap_server(
        user: &str,
        password: &str,
    ) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let mut mock = BootstrapMockServer::new(user, password);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_config = Arc::new(server::Config {
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap()],
            ..Default::default()
        });
        let handle = tokio::spawn(async move {
            let _ = mock.run_on_socket(server_config, &listener).await;
        });
        (addr, handle)
    }

    #[derive(Clone)]
    struct BootstrapMockServer {
        expected_user: String,
        expected_password: String,
        installed_keys: Arc<StdMutex<Vec<String>>>,
        password_disabled: Arc<AtomicBool>,
        reject_publickey_login: bool,
        commands: Arc<StdMutex<Vec<String>>>,
    }

    impl BootstrapMockServer {
        fn new(user: &str, password: &str) -> Self {
            Self {
                expected_user: user.to_string(),
                expected_password: password.to_string(),
                installed_keys: Arc::new(StdMutex::new(Vec::new())),
                password_disabled: Arc::new(AtomicBool::new(false)),
                reject_publickey_login: false,
                commands: Arc::new(StdMutex::new(Vec::new())),
            }
        }
    }

    impl server::Server for BootstrapMockServer {
        type Handler = Self;

        fn new_client(&mut self, _peer_addr: Option<SocketAddr>) -> Self::Handler {
            self.clone()
        }
    }

    impl server::Handler for BootstrapMockServer {
        type Error = russh::Error;

        async fn auth_password(
            &mut self,
            user: &str,
            password: &str,
        ) -> std::result::Result<Auth, Self::Error> {
            if self.password_disabled.load(Ordering::SeqCst) {
                return Ok(Auth::reject());
            }
            if user == self.expected_user && password == self.expected_password {
                Ok(Auth::Accept)
            } else {
                Ok(Auth::reject())
            }
        }

        async fn auth_publickey(
            &mut self,
            user: &str,
            public_key: &PublicKey,
        ) -> std::result::Result<Auth, Self::Error> {
            if self.reject_publickey_login || user != self.expected_user {
                return Ok(Auth::reject());
            }
            let offered_fp = format!("{}", public_key.fingerprint(HashAlg::Sha256));
            let accepted = self
                .installed_keys
                .lock()
                .unwrap()
                .iter()
                .filter_map(|line| PublicKey::from_openssh(line).ok())
                .any(|installed| {
                    format!("{}", installed.fingerprint(HashAlg::Sha256)) == offered_fp
                });
            if accepted {
                Ok(Auth::Accept)
            } else {
                Ok(Auth::reject())
            }
        }

        async fn channel_open_session(
            &mut self,
            _channel: Channel<ServerMsg>,
            reply: server::ChannelOpenHandle,
            _session: &mut Session,
        ) -> std::result::Result<(), Self::Error> {
            reply.accept().await;
            Ok(())
        }

        async fn exec_request(
            &mut self,
            channel: ChannelId,
            data: &[u8],
            session: &mut Session,
        ) -> std::result::Result<(), Self::Error> {
            let cmd = String::from_utf8_lossy(data).to_string();
            self.commands.lock().unwrap().push(cmd.clone());
            session.channel_success(channel)?;

            let mut exit_code = 0u32;
            if cmd.contains("authorized_keys") {
                if let Some(key_line) = cmd.rsplit('\'').nth(1) {
                    let mut guard = self.installed_keys.lock().unwrap();
                    if !guard.iter().any(|existing| existing == key_line) {
                        guard.push(key_line.to_string());
                    }
                } else {
                    exit_code = 1;
                }
            } else if cmd.contains("PasswordAuthentication no") {
                self.password_disabled.store(true, Ordering::SeqCst);
            }

            session.exit_status_request(channel, exit_code)?;
            session.eof(channel)?;
            session.close(channel)?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn bootstrap_generates_ed25519_key_verifies_second_login_and_swaps_credential() {
        let mut mock = BootstrapMockServer::new("root", "initial-vps-pass");
        let mock_state = mock.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_config = Arc::new(server::Config {
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap()],
            ..Default::default()
        });
        let server_task =
            tokio::spawn(async move { mock.run_on_socket(server_config, &listener).await });

        let db = Catalog::in_memory().await.unwrap();
        let master_key = Arc::new(MasterKey::from_bytes([21; 32]));
        let old_cred_id = "old-pass-cred";
        let old_cred = db
            .add_credential(NewCredential {
                id: Some(old_cred_id.to_string()),
                name: "temp-root-password".to_string(),
                username: "root".to_string(),
                auth_type: AuthType::Password,
                password_enc: Some(
                    encrypt_envelope(
                        &master_key,
                        &format!("{old_cred_id}:password"),
                        b"initial-vps-pass",
                    )
                    .unwrap(),
                ),
                private_key_enc: None,
                passphrase_enc: None,
            })
            .await
            .unwrap();

        let mut new_asset =
            NewAsset::new("dmit-lax", addr.ip().to_string(), i64::from(addr.port()));
        new_asset.credential_id = Some(old_cred.id.clone());
        let asset = db.add_asset(new_asset).await.unwrap();

        let mut harden_opts = BootstrapKeyOptions::default();
        harden_opts.disable_password_auth = true;
        let outcome = bootstrap_asset_ssh_key(
            &db,
            Arc::clone(&master_key),
            &asset.id,
            harden_opts,
            Duration::from_secs(5),
        )
        .await
        .unwrap();

        assert!(outcome.verified_key_login);
        assert!(outcome.pruned_previous_credential);
        assert!(outcome.password_auth_disabled);
        assert!(outcome.warnings.is_empty());
        assert!(outcome.public_key.starts_with("ssh-ed25519 "));
        assert_eq!(outcome.credential.auth_type, "key");
        assert_eq!(
            outcome.asset.credential_id.as_deref(),
            Some(outcome.credential.id.as_str())
        );
        assert!(
            db.get_credential(&old_cred.id).await.unwrap().is_none(),
            "unused previous password credential should be pruned atomically"
        );
        assert!(mock_state.password_disabled.load(Ordering::SeqCst));
        assert_eq!(mock_state.installed_keys.lock().unwrap().len(), 1);

        // Rotating a second time over the newly bound Ed25519 key (with password auth now disabled!)
        // must also succeed with zero credential name collision!
        let rotated = bootstrap_asset_ssh_key(
            &db,
            Arc::clone(&master_key),
            &asset.id,
            BootstrapKeyOptions::default(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_ne!(rotated.credential.id, outcome.credential.id);
        assert!(rotated.pruned_previous_credential);
        assert_eq!(mock_state.installed_keys.lock().unwrap().len(), 2);

        server_task.abort();
    }

    #[tokio::test]
    async fn bootstrap_aborts_without_mutating_catalog_when_second_connection_fails() {
        let mut mock = BootstrapMockServer::new("ubuntu", "secret-pass");
        mock.reject_publickey_login = true;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_config = Arc::new(server::Config {
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).unwrap()],
            ..Default::default()
        });
        let server_task =
            tokio::spawn(async move { mock.run_on_socket(server_config, &listener).await });

        let db = Catalog::in_memory().await.unwrap();
        let master_key = Arc::new(MasterKey::from_bytes([22; 32]));
        let asset = db
            .add_asset(NewAsset::new(
                "unmanaged-vps",
                addr.ip().to_string(),
                i64::from(addr.port()),
            ))
            .await
            .unwrap();

        let mut fail_opts = BootstrapKeyOptions::default();
        fail_opts.username = Some("ubuntu".to_string());
        fail_opts.password = Some("secret-pass".to_string());
        let err = bootstrap_asset_ssh_key(
            &db,
            Arc::clone(&master_key),
            &asset.id,
            fail_opts,
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("second-connection verification"),
            "unexpected error: {err}"
        );

        let unchanged = db.get_asset_by_id(&asset.id).await.unwrap().unwrap();
        assert!(unchanged.credential_id.is_none());
        assert!(db.list_credentials().await.unwrap().is_empty());

        server_task.abort();
    }
}
