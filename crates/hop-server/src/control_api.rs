use std::{collections::BTreeMap, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{bail, Context, Result};
use axum::{
    extract::{Path as AxumPath, Query, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use hop_core::{
    encrypt_envelope, new_id, Asset, AssetAccessMode, AssetProbeSample, AuditEvent, AuthType,
    Catalog, CatalogError, CatalogErrorCode, Credential, HopCoreError, KnownHost, MasterKey,
    NewAsset, NewAuditEvent, NewAuthorizedKey, NewCredential, NewNotificationChannel,
    NotificationChannel, NotificationChannelKind, ProbeSettings, ResourceOwnership, Session,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::watch;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::{
    bootstrap::{bootstrap_asset_ssh_key, BootstrapKeyOptions},
    notify::{
        build_encrypted_notification_config, decrypt_notification_config,
        send_channel_notification, summarize_notification_config, AlertEvent,
        NotificationConfigInput, NotificationConfigSummary,
    },
    ssh::session_registry::{ActiveSessionRegistry, TerminateSessionResult},
};

#[derive(Clone)]
pub struct ControlApiState {
    catalog: Catalog,
    master_key: Arc<MasterKey>,
    token_hash: [u8; 32],
    active_sessions: ActiveSessionRegistry,
    ssh_listen: String,
    probe_timeout: Duration,
    scheduler_reload: Option<watch::Sender<u64>>,
}

impl ControlApiState {
    pub fn new(
        catalog: Catalog,
        master_key: Arc<MasterKey>,
        token: &str,
        active_sessions: ActiveSessionRegistry,
    ) -> Result<Self> {
        let token = token.trim();
        if token.is_empty() {
            bail!("Control API token must not be empty");
        }
        Ok(Self {
            catalog,
            master_key,
            token_hash: Sha256::digest(token.as_bytes()).into(),
            active_sessions,
            ssh_listen: "0.0.0.0:2222".to_string(),
            probe_timeout: Duration::from_secs(4),
            scheduler_reload: None,
        })
    }

    pub fn with_ssh_listen(mut self, ssh_listen: impl Into<String>) -> Self {
        self.ssh_listen = ssh_listen.into();
        self
    }

    pub fn with_probe_timeout(mut self, timeout: Duration) -> Self {
        self.probe_timeout = timeout;
        self
    }

    pub fn with_scheduler_reload(mut self, reload: watch::Sender<u64>) -> Self {
        self.scheduler_reload = Some(reload);
        self
    }
}

pub async fn serve_with_shutdown(
    bind: SocketAddr,
    state: ControlApiState,
    cors_allowlist: &[String],
    shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("bind Control API at {bind}"))?;
    axum::serve(listener, router(state).layer(cors_layer(cors_allowlist)?))
        .with_graceful_shutdown(wait_for_shutdown(shutdown))
        .await?;
    Ok(())
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    let _ = shutdown.changed().await;
}

fn cors_layer(cors_allowlist: &[String]) -> Result<CorsLayer> {
    let mut cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);
    match cors_allowlist {
        [] => {}
        [origin] if origin == "*" => {
            cors = cors.allow_origin(AllowOrigin::any());
        }
        origins => {
            if origins.iter().any(|origin| origin == "*") {
                bail!("api.cors_allowlist wildcard '*' must be the only entry");
            }
            let origins = origins
                .iter()
                .map(|origin| {
                    HeaderValue::from_str(origin)
                        .with_context(|| format!("invalid api.cors_allowlist origin {origin}"))
                })
                .collect::<Result<Vec<_>>>()?;
            cors = cors.allow_origin(AllowOrigin::list(origins));
        }
    }
    Ok(cors)
}

pub fn router(state: ControlApiState) -> Router {
    Router::new()
        .route("/api/v1/status", get(status))
        .route("/api/v1/assets", get(assets).post(create_asset))
        .route("/api/v1/assets/probe", post(probe_all_assets))
        .route("/api/v1/assets/probe-history", get(probe_history_all))
        .route(
            "/api/v1/assets/{id}",
            put(update_asset).delete(delete_asset),
        )
        .route("/api/v1/assets/{id}/probe", post(probe_single_asset))
        .route(
            "/api/v1/assets/{id}/probe-history",
            get(probe_history_single),
        )
        .route(
            "/api/v1/assets/{id}/bootstrap-key",
            post(bootstrap_asset_key),
        )
        .route(
            "/api/v1/probe/settings",
            get(get_probe_settings).put(update_probe_settings),
        )
        .route(
            "/api/v1/notifications/channels",
            get(notification_channels).post(create_notification_channel),
        )
        .route(
            "/api/v1/notifications/channels/{id}",
            put(update_notification_channel).delete(delete_notification_channel),
        )
        .route(
            "/api/v1/notifications/channels/{id}/test",
            post(test_notification_channel),
        )
        .route(
            "/api/v1/credentials",
            get(credentials).post(create_credential),
        )
        .route(
            "/api/v1/credentials/{id}",
            put(update_credential).delete(delete_credential),
        )
        .route(
            "/api/v1/access-keys",
            get(access_keys).post(create_access_key),
        )
        .route(
            "/api/v1/access-keys/{id}",
            axum::routing::delete(delete_access_key),
        )
        .route(
            "/api/v1/access-keys/{id}/enabled",
            put(set_access_key_enabled),
        )
        .route(
            "/api/v1/access-keys/{id}/access",
            put(set_access_key_access),
        )
        .route("/api/v1/sessions", get(sessions))
        .route("/api/v1/sessions/{id}/terminate", post(terminate_session))
        .route(
            "/api/v1/known-hosts",
            get(known_hosts).delete(reset_known_host),
        )
        .route("/api/v1/audit-events", get(audit_events))
        .route("/api/v1/catalog/revision", get(revision))
        .with_state(state)
}

async fn status(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<StatusResponse>> {
    authenticate(&state, &headers)?;
    let catalog = state.catalog.status().await?;
    Ok(Json(StatusResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        catalog_revision: catalog.revision,
        ssh_listen: state.ssh_listen.clone(),
    }))
}

async fn revision(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<RevisionResponse>> {
    authenticate(&state, &headers)?;
    let revision = state.catalog.revision().await.map_err(ApiError::internal)?;
    Ok(Json(RevisionResponse { revision }))
}

async fn assets(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<AssetView>>> {
    authenticate(&state, &headers)?;
    let assets = state
        .catalog
        .list_assets()
        .await
        .map_err(ApiError::internal)?;
    let mut views = Vec::with_capacity(assets.len());
    for asset in assets {
        views.push(asset_view(&state.catalog, asset).await?);
    }
    Ok(Json(views))
}

async fn create_asset(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    Json(request): Json<AssetWriteRequest>,
) -> ApiResult<(StatusCode, Json<AssetView>)> {
    authenticate(&state, &headers)?;
    let asset = state.catalog.add_asset(request.into()).await?;
    record_api_audit(
        &state.catalog,
        "asset.create",
        "asset",
        Some(&asset.id),
        Some(&asset.name),
        Some(serde_json::json!({
            "protocol": asset.protocol,
            "hostname": asset.hostname,
            "port": asset.port,
        })),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(asset_view(&state.catalog, asset).await?),
    ))
}

async fn update_asset(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<AssetWriteRequest>,
) -> ApiResult<Json<AssetView>> {
    authenticate(&state, &headers)?;
    state.catalog.update_asset(&id, request.into()).await?;
    let asset = state
        .catalog
        .get_asset_by_id(&id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    record_api_audit(
        &state.catalog,
        "asset.update",
        "asset",
        Some(&asset.id),
        Some(&asset.name),
        None,
    )
    .await;
    Ok(Json(asset_view(&state.catalog, asset).await?))
}

async fn delete_asset(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    authenticate(&state, &headers)?;
    let existing = state.catalog.get_asset_by_id(&id).await?;
    state.catalog.delete_asset(&id).await?;
    record_api_audit(
        &state.catalog,
        "asset.delete",
        "asset",
        Some(&id),
        existing.as_ref().map(|asset| asset.name.as_str()),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn probe_all_assets(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<AssetView>>> {
    authenticate(&state, &headers)?;
    let probed = crate::probe::probe_all_assets(
        &state.catalog,
        Some(Arc::clone(&state.master_key)),
        state.probe_timeout,
    )
    .await
    .map_err(ApiError::internal)?;
    let healthy = probed
        .iter()
        .filter(|asset| asset.health.status == "healthy")
        .count();
    record_api_audit(
        &state.catalog,
        "asset.probe_all",
        "asset",
        None,
        Some("fleet"),
        Some(serde_json::json!({
            "total": probed.len(),
            "healthy": healthy,
        })),
    )
    .await;
    let mut views = Vec::with_capacity(probed.len());
    for asset in probed {
        views.push(asset_view(&state.catalog, asset).await?);
    }
    Ok(Json(views))
}

async fn probe_single_asset(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<AssetView>> {
    authenticate(&state, &headers)?;
    let existing = state
        .catalog
        .get_asset_by_id(&id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let probed = crate::probe::probe_asset(
        &state.catalog,
        Some(&state.master_key),
        &existing,
        state.probe_timeout,
    )
    .await
    .map_err(ApiError::internal)?;
    record_api_audit(
        &state.catalog,
        "asset.probe",
        "asset",
        Some(&probed.id),
        Some(&probed.name),
        Some(serde_json::json!({
            "status": probed.health.status,
            "latency_ms": probed.health.latency_ms,
        })),
    )
    .await;
    Ok(Json(asset_view(&state.catalog, probed).await?))
}

async fn credentials(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<CredentialView>>> {
    authenticate(&state, &headers)?;
    let credentials = state
        .catalog
        .list_credentials()
        .await
        .map_err(ApiError::internal)?;
    let mut views = Vec::with_capacity(credentials.len());
    for credential in credentials {
        views.push(credential_view(&state.catalog, credential).await?);
    }
    Ok(Json(views))
}

async fn create_credential(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    Json(request): Json<CredentialWriteRequest>,
) -> ApiResult<(StatusCode, Json<CredentialView>)> {
    authenticate(&state, &headers)?;
    let id = new_id();
    let credential = build_credential(&state.master_key, &id, request, None)?;
    let credential = state.catalog.add_credential(credential).await?;
    record_api_audit(
        &state.catalog,
        "credential.create",
        "credential",
        Some(&credential.id),
        Some(&credential.name),
        Some(serde_json::json!({ "auth_type": credential.auth_type })),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(credential_view(&state.catalog, credential).await?),
    ))
}

async fn update_credential(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<CredentialWriteRequest>,
) -> ApiResult<Json<CredentialView>> {
    authenticate(&state, &headers)?;
    let existing = state
        .catalog
        .get_credential(&id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let credential = build_credential(&state.master_key, &id, request, Some(&existing))?;
    state.catalog.update_credential(&id, credential).await?;
    let credential = state
        .catalog
        .get_credential(&id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    record_api_audit(
        &state.catalog,
        "credential.update",
        "credential",
        Some(&credential.id),
        Some(&credential.name),
        None,
    )
    .await;
    Ok(Json(credential_view(&state.catalog, credential).await?))
}

async fn delete_credential(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    authenticate(&state, &headers)?;
    let existing = state.catalog.get_credential(&id).await?;
    state.catalog.delete_credential(&id).await?;
    record_api_audit(
        &state.catalog,
        "credential.delete",
        "credential",
        Some(&id),
        existing.as_ref().map(|cred| cred.name.as_str()),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn access_keys(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<AccessKeyView>>> {
    authenticate(&state, &headers)?;
    let keys = state
        .catalog
        .list_authorized_keys()
        .await
        .map_err(ApiError::internal)?;
    let mut views = Vec::with_capacity(keys.len());
    for key in keys {
        views.push(access_key_view(&state.catalog, key).await?);
    }
    Ok(Json(views))
}

async fn create_access_key(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    Json(request): Json<AccessKeyCreateRequest>,
) -> ApiResult<(StatusCode, Json<AccessKeyView>)> {
    authenticate(&state, &headers)?;
    let (public_key, fingerprint) = crate::local_cli::parse_public_key_line(&request.public_key)
        .map_err(|_| ApiError::validation("public_key must be a valid OpenSSH public key"))?;
    let (mode, asset_ids) = access_scope(request.assets);
    let key = state
        .catalog
        .add_authorized_key_with_access(
            NewAuthorizedKey::new(request.name, public_key, fingerprint),
            mode,
            &asset_ids,
        )
        .await?;
    record_api_audit(
        &state.catalog,
        "access_key.create",
        "access_key",
        Some(&key.id),
        Some(&key.name),
        Some(serde_json::json!({ "access_mode": key.asset_access_mode })),
    )
    .await;
    let view = access_key_view(&state.catalog, key).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

async fn delete_access_key(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    authenticate(&state, &headers)?;
    let existing = state.catalog.get_authorized_key_by_id(&id).await?;
    state.catalog.delete_authorized_key(&id).await?;
    record_api_audit(
        &state.catalog,
        "access_key.delete",
        "access_key",
        Some(&id),
        existing.as_ref().map(|key| key.name.as_str()),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_access_key_enabled(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<AccessKeyEnabledRequest>,
) -> ApiResult<Json<AccessKeyView>> {
    authenticate(&state, &headers)?;
    state
        .catalog
        .set_authorized_key_active(&id, request.enabled)
        .await?;
    let key = state
        .catalog
        .get_authorized_key_by_id(&id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    record_api_audit(
        &state.catalog,
        "access_key.set_enabled",
        "access_key",
        Some(&key.id),
        Some(&key.name),
        Some(serde_json::json!({ "enabled": request.enabled })),
    )
    .await;
    Ok(Json(access_key_view(&state.catalog, key).await?))
}

async fn set_access_key_access(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<AccessKeyAccessRequest>,
) -> ApiResult<Json<AccessKeyView>> {
    authenticate(&state, &headers)?;
    let (mode, asset_ids) = access_scope(request.assets);
    state
        .catalog
        .set_authorized_key_access(&id, mode, &asset_ids)
        .await?;
    let key = state
        .catalog
        .get_authorized_key_by_id(&id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    record_api_audit(
        &state.catalog,
        "access_key.set_access",
        "access_key",
        Some(&key.id),
        Some(&key.name),
        Some(serde_json::json!({
            "access_mode": mode,
            "asset_count": asset_ids.len(),
        })),
    )
    .await;
    Ok(Json(access_key_view(&state.catalog, key).await?))
}

async fn sessions(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Session>>> {
    authenticate(&state, &headers)?;
    Ok(Json(
        state
            .catalog
            .list_sessions(100)
            .await
            .map_err(ApiError::internal)?,
    ))
}

async fn audit_events(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<AuditEvent>>> {
    authenticate(&state, &headers)?;
    Ok(Json(
        state
            .catalog
            .list_audit_events(100)
            .await
            .map_err(ApiError::internal)?,
    ))
}

async fn known_hosts(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<KnownHost>>> {
    authenticate(&state, &headers)?;
    let hosts = state
        .catalog
        .list_known_hosts()
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(hosts))
}

async fn reset_known_host(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    Json(request): Json<KnownHostResetRequest>,
) -> ApiResult<StatusCode> {
    authenticate(&state, &headers)?;
    if !request.confirm_reset {
        return Err(ApiError::validation(
            "confirm_reset must be true to reset known-host trust",
        ));
    }
    let hostname = request.hostname.trim();
    if hostname.is_empty() {
        return Err(ApiError::validation("hostname must not be empty"));
    }
    if request.key_type.trim().is_empty() {
        return Err(ApiError::validation("key_type must not be empty"));
    }
    hop_core::validate_tcp_port(request.port).map_err(ApiError::from)?;
    let deleted = state
        .catalog
        .delete_known_host(hostname, request.port, request.key_type.trim())
        .await
        .map_err(ApiError::internal)?;
    if !deleted {
        return Err(ApiError::not_found());
    }
    let target_label = format!("{hostname}:{} ({})", request.port, request.key_type.trim());
    record_api_audit(
        &state.catalog,
        "known_host.reset",
        "known_host",
        None,
        Some(&target_label),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn terminate_session(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<TerminateResponse>> {
    authenticate(&state, &headers)?;
    let terminated = match state.active_sessions.terminate(&id).await {
        TerminateSessionResult::Signaled => {
            state
                .catalog
                .finish_session(&id, "terminated", Some("terminated by control API"))
                .await
                .map_err(ApiError::internal)?;
            record_api_audit(
                &state.catalog,
                "session.terminate",
                "session",
                Some(&id),
                Some(&id),
                None,
            )
            .await;
            true
        }
        TerminateSessionResult::NotFound => false,
    };
    Ok(Json(TerminateResponse { id, terminated }))
}

#[derive(Debug, Default, Deserialize)]
struct ProbeHistoryQuery {
    limit: Option<i64>,
}

async fn probe_history_all(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    Query(query): Query<ProbeHistoryQuery>,
) -> ApiResult<Json<BTreeMap<String, Vec<AssetProbeSample>>>> {
    authenticate(&state, &headers)?;
    let limit = query.limit.unwrap_or(24).clamp(1, 96);
    let samples = state
        .catalog
        .list_recent_probe_samples(limit)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(samples))
}

async fn probe_history_single(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ProbeHistoryQuery>,
) -> ApiResult<Json<Vec<AssetProbeSample>>> {
    authenticate(&state, &headers)?;
    let _existing = state
        .catalog
        .get_asset_by_id(&id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let limit = query.limit.unwrap_or(48).clamp(1, 96);
    let samples = state
        .catalog
        .list_asset_probe_samples(&id, limit)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(samples))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapKeyRequest {
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    credential_name: Option<String>,
    #[serde(default)]
    disable_password_auth: bool,
    #[serde(default = "default_true_bool")]
    prune_previous_credential: bool,
}

#[derive(Serialize)]
struct BootstrapKeyResponse {
    asset: AssetView,
    credential: CredentialView,
    public_key: String,
    public_key_fingerprint: String,
    verified_key_login: bool,
    pruned_previous_credential: bool,
    password_auth_disabled: bool,
    warnings: Vec<String>,
}

async fn bootstrap_asset_key(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<BootstrapKeyRequest>,
) -> ApiResult<Json<BootstrapKeyResponse>> {
    authenticate(&state, &headers)?;
    let existing = state
        .catalog
        .get_asset_by_id(&id)
        .await?
        .ok_or_else(ApiError::not_found)?;

    let timeout = state.probe_timeout.max(Duration::from_secs(10));
    let outcome = match bootstrap_asset_ssh_key(
        &state.catalog,
        Arc::clone(&state.master_key),
        &existing.id,
        BootstrapKeyOptions {
            username: request.username,
            password: request.password,
            credential_name: request.credential_name,
            disable_password_auth: request.disable_password_auth,
            prune_previous_credential: request.prune_previous_credential,
        },
        timeout,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => {
            if let Some(core_err) = err.downcast_ref::<HopCoreError>() {
                return Err(ApiError::from(HopCoreError::Validation(
                    core_err.to_string(),
                )));
            }
            let message: String = format!("{err:#}").chars().take(480).collect();
            let _ = state
                .catalog
                .record_audit_event(NewAuditEvent {
                    actor_label: "api-token".to_string(),
                    action: "asset.bootstrap_key".to_string(),
                    target_type: "asset".to_string(),
                    target_id: Some(existing.id),
                    target_label: Some(existing.name),
                    result: "failed".to_string(),
                    source_ip: None,
                    details_json: Some(serde_json::json!({ "error": message }).to_string()),
                })
                .await;
            return Err(ApiError::upstream("bootstrap_failed", message));
        }
    };

    record_api_audit(
        &state.catalog,
        "asset.bootstrap_key",
        "asset",
        Some(&outcome.asset.id),
        Some(&outcome.asset.name),
        Some(serde_json::json!({
            "credential_id": outcome.credential.id,
            "fingerprint": outcome.public_key_fingerprint,
            "pruned_previous_credential": outcome.pruned_previous_credential,
            "password_auth_disabled": outcome.password_auth_disabled,
        })),
    )
    .await;

    Ok(Json(BootstrapKeyResponse {
        asset: asset_view(&state.catalog, outcome.asset).await?,
        credential: credential_view(&state.catalog, outcome.credential).await?,
        public_key: outcome.public_key,
        public_key_fingerprint: outcome.public_key_fingerprint,
        verified_key_login: outcome.verified_key_login,
        pruned_previous_credential: outcome.pruned_previous_credential,
        password_auth_disabled: outcome.password_auth_disabled,
        warnings: outcome.warnings,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeSettingsWriteRequest {
    enabled: bool,
    interval_seconds: i64,
    timeout_ms: i64,
    failure_threshold: i64,
    notify_on_recovery: bool,
}

async fn get_probe_settings(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<ProbeSettings>> {
    authenticate(&state, &headers)?;
    let settings = state
        .catalog
        .get_probe_settings()
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(settings))
}

async fn update_probe_settings(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    Json(request): Json<ProbeSettingsWriteRequest>,
) -> ApiResult<Json<ProbeSettings>> {
    authenticate(&state, &headers)?;
    let updated = state
        .catalog
        .update_probe_settings(&ProbeSettings {
            enabled: request.enabled,
            interval_seconds: request.interval_seconds,
            timeout_ms: request.timeout_ms,
            failure_threshold: request.failure_threshold,
            notify_on_recovery: request.notify_on_recovery,
            updated_at: None,
        })
        .await?;
    if let Some(reload) = &state.scheduler_reload {
        reload.send_modify(|rev| *rev = rev.wrapping_add(1));
    }
    record_api_audit(
        &state.catalog,
        "probe.settings_update",
        "probe_settings",
        Some("singleton"),
        Some("probe-scheduler"),
        Some(serde_json::json!({
            "enabled": updated.enabled,
            "interval_seconds": updated.interval_seconds,
            "timeout_ms": updated.timeout_ms,
            "failure_threshold": updated.failure_threshold,
            "notify_on_recovery": updated.notify_on_recovery,
        })),
    )
    .await;
    Ok(Json(updated))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NotificationChannelWriteRequest {
    name: String,
    kind: NotificationChannelKind,
    #[serde(default = "default_true_bool")]
    enabled: bool,
    #[serde(default)]
    config: NotificationConfigInput,
}

#[derive(Serialize)]
struct NotificationChannelView {
    id: String,
    name: String,
    kind: String,
    enabled: bool,
    config: NotificationConfigSummary,
    created_at: Option<String>,
    updated_at: Option<String>,
}

fn notification_channel_view(
    master_key: &MasterKey,
    channel: NotificationChannel,
) -> NotificationChannelView {
    let config = summarize_notification_config(master_key, &channel);
    NotificationChannelView {
        id: channel.id,
        name: channel.name,
        kind: channel.kind,
        enabled: channel.enabled,
        config,
        created_at: channel.created_at,
        updated_at: channel.updated_at,
    }
}

async fn notification_channels(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<NotificationChannelView>>> {
    authenticate(&state, &headers)?;
    let channels = state
        .catalog
        .list_notification_channels()
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(
        channels
            .into_iter()
            .map(|chan| notification_channel_view(&state.master_key, chan))
            .collect(),
    ))
}

async fn create_notification_channel(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    Json(request): Json<NotificationChannelWriteRequest>,
) -> ApiResult<(StatusCode, Json<NotificationChannelView>)> {
    authenticate(&state, &headers)?;
    let id = new_id();
    let config_enc = build_encrypted_notification_config(
        &state.master_key,
        &id,
        request.kind,
        &request.config,
        None,
    )?;
    let created = state
        .catalog
        .add_notification_channel(NewNotificationChannel {
            id: Some(id),
            name: request.name,
            kind: request.kind,
            enabled: request.enabled,
            config_enc,
        })
        .await?;
    record_api_audit(
        &state.catalog,
        "notification_channel.create",
        "notification_channel",
        Some(&created.id),
        Some(&created.name),
        Some(serde_json::json!({ "kind": created.kind, "enabled": created.enabled })),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(notification_channel_view(&state.master_key, created)),
    ))
}

async fn update_notification_channel(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<NotificationChannelWriteRequest>,
) -> ApiResult<Json<NotificationChannelView>> {
    authenticate(&state, &headers)?;
    let existing = state
        .catalog
        .get_notification_channel(&id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let decrypted_existing = decrypt_notification_config(&state.master_key, &existing).ok();
    let config_enc = build_encrypted_notification_config(
        &state.master_key,
        &id,
        request.kind,
        &request.config,
        decrypted_existing.as_ref(),
    )?;
    let updated = state
        .catalog
        .update_notification_channel(
            &id,
            NewNotificationChannel {
                id: Some(id.clone()),
                name: request.name,
                kind: request.kind,
                enabled: request.enabled,
                config_enc,
            },
        )
        .await?;
    record_api_audit(
        &state.catalog,
        "notification_channel.update",
        "notification_channel",
        Some(&updated.id),
        Some(&updated.name),
        Some(serde_json::json!({ "kind": updated.kind, "enabled": updated.enabled })),
    )
    .await;
    Ok(Json(notification_channel_view(&state.master_key, updated)))
}

async fn delete_notification_channel(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    authenticate(&state, &headers)?;
    let existing = state.catalog.get_notification_channel(&id).await?;
    let deleted = state.catalog.delete_notification_channel(&id).await?;
    if !deleted {
        return Err(ApiError::not_found());
    }
    record_api_audit(
        &state.catalog,
        "notification_channel.delete",
        "notification_channel",
        Some(&id),
        existing.as_ref().map(|chan| chan.name.as_str()),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct NotificationTestResponse {
    delivered: bool,
}

async fn test_notification_channel(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<NotificationTestResponse>> {
    authenticate(&state, &headers)?;
    let channel = state
        .catalog
        .get_notification_channel(&id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let event = AlertEvent::ChannelTest {
        channel_name: channel.name.clone(),
        channel_kind: channel.kind.clone(),
    };
    if let Err(err) = send_channel_notification(&state.master_key, &channel, &event).await {
        let msg: String = format!("{err:#}").chars().take(360).collect();
        let _ = state
            .catalog
            .record_audit_event(NewAuditEvent {
                actor_label: "api-token".to_string(),
                action: "notification_channel.test".to_string(),
                target_type: "notification_channel".to_string(),
                target_id: Some(channel.id),
                target_label: Some(channel.name),
                result: "failed".to_string(),
                source_ip: None,
                details_json: Some(serde_json::json!({ "error": msg }).to_string()),
            })
            .await;
        return Err(ApiError::upstream("notification_delivery_failed", msg));
    }
    record_api_audit(
        &state.catalog,
        "notification_channel.test",
        "notification_channel",
        Some(&channel.id),
        Some(&channel.name),
        Some(serde_json::json!({ "kind": channel.kind })),
    )
    .await;
    Ok(Json(NotificationTestResponse { delivered: true }))
}

async fn record_api_audit(
    catalog: &Catalog,
    action: &str,
    target_type: &str,
    target_id: Option<&str>,
    target_label: Option<&str>,
    details: Option<serde_json::Value>,
) {
    let _ = catalog
        .record_audit_event(NewAuditEvent {
            actor_label: "api-token".to_string(),
            action: action.to_string(),
            target_type: target_type.to_string(),
            target_id: target_id.map(ToString::to_string),
            target_label: target_label.map(ToString::to_string),
            result: "success".to_string(),
            source_ip: None,
            details_json: details.map(|value| value.to_string()),
        })
        .await;
}

fn authenticate(state: &ControlApiState, headers: &HeaderMap) -> ApiResult<()> {
    let supplied = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    let supplied_hash: [u8; 32] = Sha256::digest(supplied.as_bytes()).into();
    if bool::from(state.token_hash.ct_eq(&supplied_hash)) {
        Ok(())
    } else {
        Err(ApiError::unauthorized())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetWriteRequest {
    name: String,
    #[serde(default = "default_asset_protocol")]
    protocol: String,
    hostname: String,
    port: i64,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    credential_id: Option<String>,
    #[serde(default)]
    via_asset_id: Option<String>,
    #[serde(default = "default_true_bool")]
    allow_tcp_forwarding: bool,
}

impl From<AssetWriteRequest> for NewAsset {
    fn from(request: AssetWriteRequest) -> Self {
        Self {
            name: request.name,
            protocol: request.protocol,
            hostname: request.hostname,
            port: request.port,
            display_name: request.display_name,
            description: request.description,
            tags: request.tags,
            credential_id: request.credential_id,
            via_asset_id: request.via_asset_id,
            allow_tcp_forwarding: request.allow_tcp_forwarding,
        }
    }
}

fn default_asset_protocol() -> String {
    "ssh".to_string()
}

fn default_true_bool() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ApiAuthType {
    Password,
    Key,
    KeyPassphrase,
}

impl From<ApiAuthType> for AuthType {
    fn from(value: ApiAuthType) -> Self {
        match value {
            ApiAuthType::Password => Self::Password,
            ApiAuthType::Key => Self::Key,
            ApiAuthType::KeyPassphrase => Self::KeyWithPassphrase,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialWriteRequest {
    name: String,
    username: String,
    auth_type: ApiAuthType,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    private_key: Option<String>,
    #[serde(default)]
    passphrase: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessKeyCreateRequest {
    name: String,
    public_key: String,
    #[serde(default)]
    assets: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessKeyEnabledRequest {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessKeyAccessRequest {
    #[serde(default)]
    assets: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KnownHostResetRequest {
    hostname: String,
    port: i64,
    key_type: String,
    confirm_reset: bool,
}

#[derive(Serialize)]
struct StatusResponse {
    status: &'static str,
    version: &'static str,
    catalog_revision: i64,
    ssh_listen: String,
}

#[derive(Serialize)]
struct RevisionResponse {
    revision: i64,
}

#[derive(Serialize)]
struct AssetView {
    #[serde(flatten)]
    asset: Asset,
    #[serde(skip_serializing_if = "Option::is_none")]
    via_asset_name: Option<String>,
    ownership: ResourceOwnership,
}

async fn asset_view(catalog: &Catalog, asset: Asset) -> ApiResult<AssetView> {
    let ownership = catalog
        .resource_ownership("asset", &asset.id)
        .await
        .map_err(ApiError::internal)?;
    let via_asset_name = match asset.via_asset_id.as_deref() {
        Some(via_id) => catalog
            .get_asset_by_id(via_id)
            .await
            .map_err(ApiError::internal)?
            .map(|parent| parent.name),
        None => None,
    };
    Ok(AssetView {
        asset,
        via_asset_name,
        ownership,
    })
}

#[derive(Serialize)]
struct CredentialView {
    id: String,
    name: String,
    username: String,
    auth_type: String,
    password: SecretStatus,
    private_key: SecretStatus,
    passphrase: SecretStatus,
    ownership: ResourceOwnership,
}

async fn credential_view(catalog: &Catalog, credential: Credential) -> ApiResult<CredentialView> {
    let ownership = catalog
        .resource_ownership("credential", &credential.id)
        .await
        .map_err(ApiError::internal)?;
    Ok(CredentialView {
        id: credential.id,
        name: credential.name,
        username: credential.username,
        auth_type: credential.auth_type,
        password: SecretStatus::from(credential.password_enc.is_some()),
        private_key: SecretStatus::from(credential.private_key_enc.is_some()),
        passphrase: SecretStatus::from(credential.passphrase_enc.is_some()),
        ownership,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum SecretStatus {
    Configured,
    Missing,
}

impl From<bool> for SecretStatus {
    fn from(configured: bool) -> Self {
        if configured {
            Self::Configured
        } else {
            Self::Missing
        }
    }
}

#[derive(Serialize)]
struct AccessKeyView {
    id: String,
    name: String,
    fingerprint: String,
    enabled: bool,
    access_mode: String,
    assets: Option<Vec<String>>,
    ownership: ResourceOwnership,
}

#[derive(Serialize)]
struct TerminateResponse {
    id: String,
    terminated: bool,
}

fn build_credential(
    master_key: &MasterKey,
    id: &str,
    request: CredentialWriteRequest,
    existing: Option<&Credential>,
) -> ApiResult<NewCredential> {
    let auth_type: AuthType = request.auth_type.into();
    let keep_password = existing
        .filter(|credential| credential.auth_type == "password")
        .and_then(|credential| credential.password_enc.clone());
    let keep_private_key = existing
        .filter(|credential| credential.auth_type != "password")
        .and_then(|credential| credential.private_key_enc.clone());
    let keep_passphrase = existing
        .filter(|credential| credential.auth_type == "key+passphrase")
        .and_then(|credential| credential.passphrase_enc.clone());
    let password_enc = match auth_type {
        AuthType::Password => {
            encrypt_api_secret(master_key, id, "password", request.password)?.or(keep_password)
        }
        AuthType::Key | AuthType::KeyWithPassphrase => None,
    };
    let private_key_enc = match auth_type {
        AuthType::Password => None,
        AuthType::Key | AuthType::KeyWithPassphrase => {
            encrypt_api_secret(master_key, id, "private_key", request.private_key)?
                .or(keep_private_key)
        }
    };
    let passphrase_enc = match auth_type {
        AuthType::KeyWithPassphrase => {
            encrypt_api_secret(master_key, id, "passphrase", request.passphrase)?
                .or(keep_passphrase)
        }
        AuthType::Password | AuthType::Key => None,
    };
    Ok(NewCredential {
        id: Some(id.to_string()),
        name: request.name,
        username: request.username,
        auth_type,
        password_enc,
        private_key_enc,
        passphrase_enc,
    })
}

fn encrypt_api_secret(
    master_key: &MasterKey,
    id: &str,
    field: &str,
    value: Option<String>,
) -> ApiResult<Option<String>> {
    value
        .map(|value| {
            encrypt_envelope(master_key, &format!("{id}:{field}"), value.as_bytes())
                .map_err(ApiError::from)
        })
        .transpose()
}

fn access_scope(assets: Option<Vec<String>>) -> (AssetAccessMode, Vec<String>) {
    match assets {
        None => (AssetAccessMode::All, Vec::new()),
        Some(assets) => (AssetAccessMode::Restricted, assets),
    }
}

async fn access_key_view(
    catalog: &Catalog,
    key: hop_core::AuthorizedKey,
) -> ApiResult<AccessKeyView> {
    let assets = match key.asset_access_mode {
        AssetAccessMode::All => None,
        AssetAccessMode::Restricted => Some(catalog.list_asset_ids_for_key(&key.id).await?),
    };
    let ownership = catalog
        .resource_ownership("access_key", &key.id)
        .await
        .map_err(ApiError::internal)?;
    Ok(AccessKeyView {
        id: key.id,
        name: key.name,
        fingerprint: key.fingerprint,
        enabled: key.is_active,
        access_mode: key.asset_access_mode.to_string(),
        assets,
        ownership,
    })
}

type ApiResult<T> = std::result::Result<T, ApiError>;

struct ApiError {
    status: StatusCode,
    body: ApiErrorBody,
}

#[derive(Serialize)]
struct ApiErrorBody {
    code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    message: String,
}

impl ApiError {
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            body: ApiErrorBody {
                code: "unauthorized".to_string(),
                path: None,
                message: "a valid Bearer management token is required".to_string(),
            },
        }
    }

    fn internal(_: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: ApiErrorBody {
                code: "internal_error".to_string(),
                path: None,
                message: "Control API operation failed".to_string(),
            },
        }
    }

    fn validation(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            body: ApiErrorBody {
                code: "validation_failed".to_string(),
                path: None,
                message: message.into(),
            },
        }
    }

    fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            body: ApiErrorBody {
                code: "not_found".to_string(),
                path: None,
                message: "catalog resource was not found".to_string(),
            },
        }
    }

    fn upstream(code: &str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            body: ApiErrorBody {
                code: code.to_string(),
                path: None,
                message: message.into(),
            },
        }
    }
}

impl From<HopCoreError> for ApiError {
    fn from(error: HopCoreError) -> Self {
        match error {
            HopCoreError::Validation(message) if message.starts_with("managed_by_source:") => {
                Self {
                    status: StatusCode::CONFLICT,
                    body: ApiErrorBody {
                        code: CatalogErrorCode::ManagedBySource.to_string(),
                        path: None,
                        message,
                    },
                }
            }
            HopCoreError::Validation(message) => Self::validation(message),
            HopCoreError::Database(_) => Self {
                status: StatusCode::CONFLICT,
                body: ApiErrorBody {
                    code: "catalog_conflict".to_string(),
                    path: None,
                    message: "catalog write conflicted with an existing or referenced resource"
                        .to_string(),
                },
            },
            other => Self::internal(other),
        }
    }
}

impl From<CatalogError> for ApiError {
    fn from(error: CatalogError) -> Self {
        let status = match error.code {
            CatalogErrorCode::RevisionConflict
            | CatalogErrorCode::OwnershipConflict
            | CatalogErrorCode::ManagedBySource
            | CatalogErrorCode::ResourceInUse => StatusCode::CONFLICT,
            CatalogErrorCode::ApplyFailed => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        };
        Self {
            status,
            body: ApiErrorBody {
                code: error.code.to_string(),
                path: error.path,
                message: error.message,
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use axum::{body::Body, http::Request};
    use hop_core::{ApplyOptions, Manifest};
    use tokio::{io::AsyncWriteExt, net::TcpListener};
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn api_requires_the_single_management_token() {
        let state = ControlApiState::new(
            Catalog::in_memory().await.unwrap(),
            Arc::new(MasterKey::generate()),
            "test-token",
            ActiveSessionRegistry::default(),
        )
        .unwrap();
        let app = router(state);

        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/status")
                    .header(header::AUTHORIZATION, "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authorized.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn control_api_stops_accepting_after_shutdown_notification() {
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = probe.local_addr().unwrap();
        drop(probe);
        let state = ControlApiState::new(
            Catalog::in_memory().await.unwrap(),
            Arc::new(MasterKey::generate()),
            "test-token",
            ActiveSessionRegistry::default(),
        )
        .unwrap();
        let (shutdown_tx, shutdown) = watch::channel(false);
        let task = tokio::spawn(serve_with_shutdown(address, state, &[], shutdown));

        let mut connected = false;
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(address).await.is_ok() {
                connected = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(connected, "Control API did not bind its listener");

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("Control API did not shut down")
            .unwrap()
            .unwrap();
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
    }

    #[tokio::test]
    async fn cors_preflight_allows_local_crud_methods_for_explicit_origins() {
        let state = ControlApiState::new(
            Catalog::in_memory().await.unwrap(),
            Arc::new(MasterKey::generate()),
            "test-token",
            ActiveSessionRegistry::default(),
        )
        .unwrap();
        let app = router(state).layer(cors_layer(&["https://panel.example".to_string()]).unwrap());

        for method in [Method::PUT, Method::DELETE] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::OPTIONS)
                        .uri("/api/v1/assets/example")
                        .header(header::ORIGIN, "https://panel.example")
                        .header(header::ACCESS_CONTROL_REQUEST_METHOD, method.as_str())
                        .header(
                            header::ACCESS_CONTROL_REQUEST_HEADERS,
                            "authorization,content-type",
                        )
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
                Some(&HeaderValue::from_static("https://panel.example"))
            );
            let allowed = response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_METHODS)
                .unwrap()
                .to_str()
                .unwrap();
            assert!(allowed.split(',').any(|value| value.trim() == method));
        }
    }

    #[tokio::test]
    async fn cors_wildcard_allows_any_origin_without_panicking() {
        let state = ControlApiState::new(
            Catalog::in_memory().await.unwrap(),
            Arc::new(MasterKey::generate()),
            "test-token",
            ActiveSessionRegistry::default(),
        )
        .unwrap();
        let app = router(state).layer(cors_layer(&["*".to_string()]).unwrap());

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/api/v1/assets")
                    .header(header::ORIGIN, "https://any-origin.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, Method::GET.as_str())
                    .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static("*"))
        );
    }

    #[test]
    fn cors_wildcard_cannot_be_mixed_with_explicit_origins() {
        let error =
            cors_layer(&["*".to_string(), "https://panel.example".to_string()]).unwrap_err();
        assert!(error.to_string().contains("must be the only entry"));
    }

    #[tokio::test]
    async fn remote_config_routes_are_removed_and_credentials_never_return_secret_material() {
        let directory = tempfile::tempdir().unwrap();
        let password = directory.path().join("password");
        fs::write(&password, "api-secret-value").unwrap();
        let manifest_yaml = format!(
            "api_version: hop/v1alpha1\ncredentials:\n  root:\n    type: password\n    username: root\n    password:\n      file: {}\n",
            password.display()
        );
        let catalog = Catalog::in_memory().await.unwrap();
        let master_key = Arc::new(MasterKey::generate());
        catalog
            .apply(
                &Manifest::from_yaml(&manifest_yaml).unwrap(),
                "local-cli",
                &master_key,
                ApplyOptions::default(),
            )
            .await
            .unwrap();

        let state = ControlApiState::new(
            catalog,
            master_key,
            "test-token",
            ActiveSessionRegistry::default(),
        )
        .unwrap();
        let app = router(state);

        for removed_route in [
            "/api/v1/config/validate",
            "/api/v1/config/diff",
            "/api/v1/config/apply",
        ] {
            let response = app
                .clone()
                .oneshot(api_request(
                    Method::POST,
                    removed_route,
                    serde_json::json!({"content": manifest_yaml, "format": "yaml"}),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        let credentials = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/credentials")
                    .header(header::AUTHORIZATION, "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(credentials.status(), StatusCode::OK);
        let body = axum::body::to_bytes(credentials.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("configured"));
        assert!(!body.contains("api-secret-value"));
        assert!(!body.contains("password_enc"));
    }

    #[tokio::test]
    async fn api_local_crud_preserves_secret_and_access_scope_boundaries() {
        let catalog = Catalog::in_memory().await.unwrap();
        let state = ControlApiState::new(
            catalog.clone(),
            Arc::new(MasterKey::generate()),
            "test-token",
            ActiveSessionRegistry::default(),
        )
        .unwrap();
        let app = router(state);
        let credential = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                "/api/v1/credentials",
                serde_json::json!({
                    "name": "root",
                    "username": "root",
                    "auth_type": "password",
                    "password": "never-return-this"
                }),
            ))
            .await
            .unwrap();
        assert_eq!(credential.status(), StatusCode::CREATED);
        let credential = response_json(credential).await;
        assert_eq!(credential["password"], "configured");
        assert_eq!(credential["ownership"], "local");
        assert!(!credential.to_string().contains("never-return-this"));
        let credential_id = credential["id"].as_str().unwrap();

        let asset = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                "/api/v1/assets",
                serde_json::json!({
                    "name": "server",
                    "hostname": "192.0.2.10",
                    "port": 22,
                    "credential_id": credential_id
                }),
            ))
            .await
            .unwrap();
        assert_eq!(asset.status(), StatusCode::CREATED);
        let asset = response_json(asset).await;
        assert_eq!(asset["ownership"], "local");
        let asset_id = asset["id"].as_str().unwrap();

        let access_key = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                "/api/v1/access-keys",
                serde_json::json!({
                    "name": "laptop",
                    "public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ test",
                    "assets": []
                }),
            ))
            .await
            .unwrap();
        assert_eq!(access_key.status(), StatusCode::CREATED);
        let access_key = response_json(access_key).await;
        assert_eq!(access_key["ownership"], "local");
        assert_eq!(access_key["access_mode"], "restricted");
        assert_eq!(access_key["assets"], serde_json::json!([]));
        assert!(!access_key.to_string().contains("AAAAC3"));
        let key_id = access_key["id"].as_str().unwrap();

        let update_access = app
            .clone()
            .oneshot(api_request(
                Method::PUT,
                &format!("/api/v1/access-keys/{key_id}/access"),
                serde_json::json!({"assets": [asset_id]}),
            ))
            .await
            .unwrap();
        assert_eq!(update_access.status(), StatusCode::OK);
        assert_eq!(
            response_json(update_access).await["assets"],
            serde_json::json!([asset_id])
        );
        assert!(catalog.revision().await.unwrap() >= 4);
    }

    #[tokio::test]
    async fn access_changes_keep_existing_sessions_until_explicit_termination() {
        let catalog = Catalog::in_memory().await.unwrap();
        let key = catalog
            .add_authorized_key(NewAuthorizedKey::new(
                "laptop",
                "ssh-ed25519 AAAA-test",
                "SHA256:test",
            ))
            .await
            .unwrap();
        let asset = catalog
            .add_asset(NewAsset::new("server", "192.0.2.10", 22))
            .await
            .unwrap();
        catalog
            .set_authorized_key_access(
                &key.id,
                AssetAccessMode::Restricted,
                std::slice::from_ref(&asset.id),
            )
            .await
            .unwrap();
        let session = catalog
            .start_session(hop_core::NewSession {
                key_id: Some(key.id.clone()),
                key_finger: key.fingerprint.clone(),
                key_name: Some(key.name.clone()),
                mode: "direct".to_string(),
                asset_id: Some(asset.id.clone()),
                asset_name: Some(asset.name),
                target_host: Some(asset.hostname),
                target_port: Some(asset.port),
                client_ip: Some("127.0.0.1".to_string()),
            })
            .await
            .unwrap();
        let active_sessions = ActiveSessionRegistry::default();
        let (terminate_tx, mut terminate_rx) = tokio::sync::mpsc::unbounded_channel();
        active_sessions
            .register(session.id.clone(), terminate_tx)
            .await;
        let state = ControlApiState::new(
            catalog.clone(),
            Arc::new(MasterKey::generate()),
            "test-token",
            active_sessions,
        )
        .unwrap();
        let app = router(state);

        let narrowed = app
            .clone()
            .oneshot(api_request(
                Method::PUT,
                &format!("/api/v1/access-keys/{}/access", key.id),
                serde_json::json!({"assets": []}),
            ))
            .await
            .unwrap();
        assert_eq!(narrowed.status(), StatusCode::OK);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), terminate_rx.recv())
                .await
                .is_err()
        );
        assert_eq!(
            catalog
                .get_session(&session.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            "started"
        );

        let terminated = app
            .oneshot(api_request(
                Method::POST,
                &format!("/api/v1/sessions/{}/terminate", session.id),
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(terminated.status(), StatusCode::OK);
        assert_eq!(response_json(terminated).await["terminated"], true);
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(1), terminate_rx.recv())
                .await
                .unwrap(),
            Some(())
        );
        assert_eq!(
            catalog
                .get_session(&session.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            "terminated"
        );
    }

    #[tokio::test]
    async fn known_host_trust_reset_requires_confirmation_and_exact_identity() {
        let catalog = Catalog::in_memory().await.unwrap();
        for (key_type, fingerprint) in
            [("ssh-ed25519", "SHA256:ed25519"), ("ssh-rsa", "SHA256:rsa")]
        {
            catalog
                .upsert_known_host(hop_core::NewKnownHost {
                    hostname: "192.0.2.10".to_string(),
                    port: 22,
                    key_type: key_type.to_string(),
                    fingerprint: fingerprint.to_string(),
                })
                .await
                .unwrap();
        }
        let state = ControlApiState::new(
            catalog.clone(),
            Arc::new(MasterKey::generate()),
            "test-token",
            ActiveSessionRegistry::default(),
        )
        .unwrap();
        let app = router(state);

        let listed = app
            .clone()
            .oneshot(api_request(
                Method::GET,
                "/api/v1/known-hosts",
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        assert_eq!(response_json(listed).await.as_array().unwrap().len(), 2);

        let rejected = app
            .clone()
            .oneshot(api_request(
                Method::DELETE,
                "/api/v1/known-hosts",
                serde_json::json!({
                    "hostname": "192.0.2.10",
                    "port": 22,
                    "key_type": "ssh-ed25519",
                    "confirm_reset": false
                }),
            ))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(catalog
            .get_known_host("192.0.2.10", 22, "ssh-ed25519")
            .await
            .unwrap()
            .is_some());

        let reset = app
            .clone()
            .oneshot(api_request(
                Method::DELETE,
                "/api/v1/known-hosts",
                serde_json::json!({
                    "hostname": "192.0.2.10",
                    "port": 22,
                    "key_type": "ssh-ed25519",
                    "confirm_reset": true
                }),
            ))
            .await
            .unwrap();
        assert_eq!(reset.status(), StatusCode::NO_CONTENT);
        assert!(catalog
            .get_known_host("192.0.2.10", 22, "ssh-ed25519")
            .await
            .unwrap()
            .is_none());
        assert!(catalog
            .get_known_host("192.0.2.10", 22, "ssh-rsa")
            .await
            .unwrap()
            .is_some());

        let missing = app
            .oneshot(api_request(
                Method::DELETE,
                "/api/v1/known-hosts",
                serde_json::json!({
                    "hostname": "192.0.2.10",
                    "port": 22,
                    "key_type": "ssh-ed25519",
                    "confirm_reset": true
                }),
            ))
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn api_crud_rejects_declarative_resources_as_managed_by_source() {
        let catalog = Catalog::in_memory().await.unwrap();
        let manifest = Manifest::from_yaml(
            "api_version: hop/v1alpha1\nassets:\n  managed:\n    type: tcp\n    host: 192.0.2.20\n    port: 3389\n",
        )
        .unwrap();
        catalog
            .apply(
                &manifest,
                "home",
                &MasterKey::generate(),
                ApplyOptions::default(),
            )
            .await
            .unwrap();
        let asset = catalog.list_assets().await.unwrap().remove(0);
        let state = ControlApiState::new(
            catalog.clone(),
            Arc::new(MasterKey::generate()),
            "test-token",
            ActiveSessionRegistry::default(),
        )
        .unwrap();
        let app = router(state);
        let listed = app
            .clone()
            .oneshot(api_request(
                Method::GET,
                "/api/v1/assets",
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(response_json(listed).await[0]["ownership"], "config");

        let response = app
            .oneshot(api_request(
                Method::PUT,
                &format!("/api/v1/assets/{}", asset.id),
                serde_json::json!({
                    "name": "managed",
                    "protocol": "tcp",
                    "hostname": "192.0.2.21",
                    "port": 3389
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response_json(response).await["code"],
            CatalogErrorCode::ManagedBySource.to_string()
        );
    }

    #[tokio::test]
    async fn status_probe_and_audit_endpoints_expose_ssh_listen_latency_and_mutations() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for _ in 0..2 {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let _ = socket.write_all(b"SSH-2.0-OpenSSH_9.7\r\n").await;
                }
            }
        });

        let catalog = Catalog::in_memory().await.unwrap();
        let state = ControlApiState::new(
            catalog.clone(),
            Arc::new(MasterKey::generate()),
            "test-token",
            ActiveSessionRegistry::default(),
        )
        .unwrap()
        .with_ssh_listen("0.0.0.0:22022")
        .with_probe_timeout(Duration::from_secs(2));
        let app = router(state);

        let status = app
            .clone()
            .oneshot(api_request(
                Method::GET,
                "/api/v1/status",
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(status.status(), StatusCode::OK);
        assert_eq!(response_json(status).await["ssh_listen"], "0.0.0.0:22022");

        let created = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                "/api/v1/assets",
                serde_json::json!({
                    "name": "hk-cn2",
                    "display_name": "HK CN2 GIA",
                    "protocol": "ssh",
                    "hostname": addr.ip().to_string(),
                    "port": addr.port(),
                    "tags": ["hk", "cn2"]
                }),
            ))
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = response_json(created).await;
        assert_eq!(created["display_name"], "HK CN2 GIA");
        assert_eq!(created["health"]["status"], "unknown");
        let asset_id = created["id"].as_str().unwrap();

        let single_probe = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                &format!("/api/v1/assets/{asset_id}/probe"),
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(single_probe.status(), StatusCode::OK);
        let single_probe = response_json(single_probe).await;
        assert_eq!(single_probe["health"]["status"], "healthy");
        assert!(single_probe["health"]["latency_ms"].is_number());

        let batch_probe = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                "/api/v1/assets/probe",
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(batch_probe.status(), StatusCode::OK);
        let batch_probe = response_json(batch_probe).await;
        assert_eq!(batch_probe[0]["health"]["status"], "healthy");

        let audits = app
            .oneshot(api_request(
                Method::GET,
                "/api/v1/audit-events",
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(audits.status(), StatusCode::OK);
        let audits = response_json(audits).await;
        let actions = audits
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["action"].as_str())
            .collect::<Vec<_>>();
        assert!(actions.contains(&"asset.create"));
        assert!(actions.contains(&"asset.probe"));
        assert!(actions.contains(&"asset.probe_all"));
    }

    #[tokio::test]
    async fn v0_3_0_probe_history_settings_notifications_and_bootstrap_api_roundtrip() {
        let (ssh_addr, ssh_server_task) =
            crate::bootstrap::tests::spawn_test_bootstrap_server("root", "one-time-pass").await;

        let webhook_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let webhook_addr = webhook_listener.local_addr().unwrap();
        let (webhook_tx, webhook_rx) = tokio::sync::oneshot::channel::<String>();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = webhook_listener.accept().await {
                let raw = crate::notify::tests::read_http_request_for_test(&mut stream).await;
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
                let _ = stream.shutdown().await;
                let _ = webhook_tx.send(raw);
            }
        });

        let catalog = Catalog::in_memory().await.unwrap();
        let (reload_tx, reload_rx) = watch::channel(0_u64);
        let state = ControlApiState::new(
            catalog.clone(),
            Arc::new(MasterKey::generate()),
            "test-token",
            ActiveSessionRegistry::default(),
        )
        .unwrap()
        .with_probe_timeout(Duration::from_secs(4))
        .with_scheduler_reload(reload_tx);
        let app = router(state);

        // 1. Create relay + chained target asset with bootstrap-key
        let relay_res = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                "/api/v1/assets",
                serde_json::json!({
                    "name": "hk-relay",
                    "protocol": "ssh",
                    "hostname": ssh_addr.ip().to_string(),
                    "port": ssh_addr.port()
                }),
            ))
            .await
            .unwrap();
        assert_eq!(relay_res.status(), StatusCode::CREATED);
        let relay = response_json(relay_res).await;
        let relay_id = relay["id"].as_str().unwrap().to_string();
        assert_eq!(relay["allow_tcp_forwarding"], true);

        let bootstrap_res = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                &format!("/api/v1/assets/{relay_id}/bootstrap-key"),
                serde_json::json!({
                    "username": "root",
                    "password": "one-time-pass"
                }),
            ))
            .await
            .unwrap();
        assert_eq!(bootstrap_res.status(), StatusCode::OK);
        let bootstrap_body = response_json(bootstrap_res).await;
        assert_eq!(bootstrap_body["verified_key_login"], true);
        assert!(bootstrap_body["public_key"]
            .as_str()
            .unwrap()
            .starts_with("ssh-ed25519 "));
        assert_eq!(bootstrap_body["credential"]["auth_type"], "key");
        assert_eq!(
            bootstrap_body["asset"]["credential_id"],
            bootstrap_body["credential"]["id"]
        );

        let leaf_res = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                "/api/v1/assets",
                serde_json::json!({
                    "name": "us-origin",
                    "protocol": "ssh",
                    "hostname": "10.20.0.8",
                    "port": 22,
                    "via_asset_id": relay_id,
                    "allow_tcp_forwarding": false
                }),
            ))
            .await
            .unwrap();
        assert_eq!(leaf_res.status(), StatusCode::CREATED);
        let leaf = response_json(leaf_res).await;
        assert_eq!(leaf["via_asset_name"], "hk-relay");
        assert_eq!(leaf["allow_tcp_forwarding"], false);

        // 2. Probe relay & verify probe-history endpoints
        let probe_res = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                &format!("/api/v1/assets/{relay_id}/probe"),
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(probe_res.status(), StatusCode::OK);

        let all_hist = app
            .clone()
            .oneshot(api_request(
                Method::GET,
                "/api/v1/assets/probe-history?limit=12",
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(all_hist.status(), StatusCode::OK);
        let all_hist_json = response_json(all_hist).await;
        assert_eq!(all_hist_json[&relay_id].as_array().unwrap().len(), 2);

        let single_hist = app
            .clone()
            .oneshot(api_request(
                Method::GET,
                &format!("/api/v1/assets/{relay_id}/probe-history?limit=12"),
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(single_hist.status(), StatusCode::OK);
        assert_eq!(
            response_json(single_hist).await.as_array().unwrap().len(),
            2
        );

        // 3. Probe settings GET / PUT + validation + reload watch signal
        let get_settings = app
            .clone()
            .oneshot(api_request(
                Method::GET,
                "/api/v1/probe/settings",
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(get_settings.status(), StatusCode::OK);
        assert_eq!(response_json(get_settings).await["enabled"], false);

        let invalid_settings = app
            .clone()
            .oneshot(api_request(
                Method::PUT,
                "/api/v1/probe/settings",
                serde_json::json!({
                    "enabled": true,
                    "interval_seconds": 5,
                    "timeout_ms": 4000,
                    "failure_threshold": 2,
                    "notify_on_recovery": true
                }),
            ))
            .await
            .unwrap();
        assert_eq!(invalid_settings.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let put_settings = app
            .clone()
            .oneshot(api_request(
                Method::PUT,
                "/api/v1/probe/settings",
                serde_json::json!({
                    "enabled": true,
                    "interval_seconds": 180,
                    "timeout_ms": 3500,
                    "failure_threshold": 3,
                    "notify_on_recovery": true
                }),
            ))
            .await
            .unwrap();
        assert_eq!(put_settings.status(), StatusCode::OK);
        assert_eq!(*reload_rx.borrow(), 1);

        // 4. Notification channels CRUD + secret redaction + test push
        let create_chan = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                "/api/v1/notifications/channels",
                serde_json::json!({
                    "name": "Ops Webhook",
                    "kind": "webhook",
                    "enabled": true,
                    "config": {
                        "url": format!("http://{webhook_addr}/alert"),
                        "secret": "super-secret-hmac-key"
                    }
                }),
            ))
            .await
            .unwrap();
        assert_eq!(create_chan.status(), StatusCode::CREATED);
        let chan_json = response_json(create_chan).await;
        let chan_id = chan_json["id"].as_str().unwrap().to_string();
        assert_eq!(chan_json["config"]["has_secret"], true);
        assert!(
            !chan_json.to_string().contains("super-secret-hmac-key"),
            "notification secret must never be returned by Control API"
        );

        // Update name while omitting secret -> existing secret must be preserved!
        let update_chan = app
            .clone()
            .oneshot(api_request(
                Method::PUT,
                &format!("/api/v1/notifications/channels/{chan_id}"),
                serde_json::json!({
                    "name": "Primary Webhook",
                    "kind": "webhook",
                    "enabled": true,
                    "config": {
                        "url": format!("http://{webhook_addr}/alert")
                    }
                }),
            ))
            .await
            .unwrap();
        assert_eq!(update_chan.status(), StatusCode::OK);
        assert_eq!(
            response_json(update_chan).await["config"]["has_secret"],
            true
        );

        let test_push = app
            .clone()
            .oneshot(api_request(
                Method::POST,
                &format!("/api/v1/notifications/channels/{chan_id}/test"),
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(test_push.status(), StatusCode::OK);
        assert_eq!(response_json(test_push).await["delivered"], true);

        let raw_http = tokio::time::timeout(Duration::from_secs(3), webhook_rx)
            .await
            .unwrap()
            .unwrap();
        assert!(
            raw_http
                .to_ascii_lowercase()
                .contains("x-hop-signature-256: sha256="),
            "preserved HMAC secret should sign test webhook: {raw_http}"
        );

        let del_chan = app
            .oneshot(api_request(
                Method::DELETE,
                &format!("/api/v1/notifications/channels/{chan_id}"),
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(del_chan.status(), StatusCode::NO_CONTENT);

        ssh_server_task.abort();
    }

    fn api_request(method: Method, uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, "Bearer test-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn response_json(response: Response) -> serde_json::Value {
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }
}
