use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path as StdPath, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{ensure, Result};
use axum::{
    body::Bytes,
    extract::{ConnectInfo, Form, Multipart, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Extension, Router,
};
use hop_core::{
    encrypt_envelope, new_id, protocol_supports_managed_credentials, validate_credential_material,
    validate_tcp_port, AssetAccessMode, AuthType, HopDb, MasterKey, NewAdminUser, NewAsset,
    NewAuditEvent, NewAuthorizedKey, NewCredential, ASSET_PROTOCOL_SSH,
};
use serde::Deserialize;
use serde_json::Value;
use tokio::{
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing::{info, warn};

use crate::ssh::session_registry::{
    ActiveSessionRegistry, TerminateSessionResult, TERMINATED_BY_ADMIN,
};

use super::{
    auth::{
        clear_cookie, cookie_token, profile_has_capability, require_login, session_cookie,
        AdminCapability, AdminSessions, AuthenticatedSession,
    },
    bootstrap, html,
    i18n::{l10n, locale_from_code, resolve_locale, L10n, LOCALE_COOKIE},
    local_cli::parse_public_key_line,
    transfer::{self, ConflictPolicy, ImportSummary, TransferFormat, TransferKind},
};

#[derive(Clone)]
pub struct AdminState {
    pub db: HopDb,
    pub master_key: Arc<MasterKey>,
    pub active_sessions: ActiveSessionRegistry,
    pub sessions: AdminSessions,
    pub ssh_port: u16,
    pub ssh_bind: SocketAddr,
    pub admin_bind: SocketAddr,
    pub started_at: String,
    pub started_instant: Instant,
    pub cookie_secure: bool,
}

pub async fn serve_admin(
    bind: SocketAddr,
    ssh_bind: SocketAddr,
    db: HopDb,
    master_key: Arc<MasterKey>,
    active_sessions: ActiveSessionRegistry,
    cookie_secure: bool,
) -> Result<()> {
    let state = AdminState {
        db,
        master_key,
        active_sessions,
        sessions: AdminSessions::default(),
        ssh_port: ssh_bind.port(),
        ssh_bind,
        admin_bind: bind,
        started_at: chrono::Utc::now().to_rfc3339(),
        started_instant: Instant::now(),
        cookie_secure,
    };
    let app = admin_router(state);

    let listener = TcpListener::bind(bind).await?;
    info!(%bind, "admin web listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn admin_router(state: AdminState) -> Router {
    admin_router_with_static_dir(state, admin_static_dist_dir())
}

fn admin_static_dist_dir() -> PathBuf {
    if let Some(static_dir) = std::env::var_os("HOP_ADMIN_STATIC_DIR") {
        return PathBuf::from(static_dir);
    }

    StdPath::new(env!("CARGO_MANIFEST_DIR")).join("../../web/admin/dist")
}

fn admin_router_with_static_dir(state: AdminState, static_dir: impl Into<PathBuf>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/login", get(login_page).post(login))
        .route("/logout", get(logout))
        .route("/set-language", get(set_language))
        .route("/assets", get(assets).post(create_asset))
        .route("/assets/export", get(export_assets))
        .route("/assets/bulk-tags", post(bulk_update_asset_tags))
        .route("/assets/{id}/edit", get(edit_asset))
        .route("/assets/{id}", post(update_asset))
        .route("/assets/{id}/delete", post(delete_asset))
        .route("/credentials", get(credentials).post(create_credential))
        .route("/credentials/export", get(export_credentials))
        .route("/credentials/{id}/edit", get(edit_credential))
        .route("/credentials/{id}", post(update_credential))
        .route("/credentials/{id}/delete", post(delete_credential))
        .route("/keys", get(keys).post(create_key))
        .route("/keys/{id}/edit", get(edit_key))
        .route("/keys/{id}", post(update_key))
        .route("/keys/{id}/deactivate", post(deactivate_key))
        .route("/keys/{id}/activate", post(activate_key))
        .route("/keys/{id}/delete", post(delete_key))
        .route("/known-hosts", get(known_hosts))
        .route(
            "/known-hosts/{hostname}/{port}/delete",
            post(delete_known_host),
        )
        .route("/sessions", get(sessions))
        .route("/sessions/terminate-all", post(terminate_all_sessions))
        .route("/sessions/{id}/terminate", post(terminate_session))
        .route("/import", get(import_page).post(import_data))
        .route("/settings", get(settings).post(update_settings))
        .route("/settings/admins", post(create_admin_user))
        .route(
            "/settings/admins/{id}/access",
            post(update_admin_user_access),
        )
        .nest_service("/admin-static", ServeDir::new(static_dir.into()))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn guard(
    headers: &HeaderMap,
    state: &AdminState,
) -> std::result::Result<AuthenticatedSession, Response> {
    require_login(headers, &state.sessions)
        .await
        .ok_or_else(|| Redirect::to("/login").into_response())
}

async fn csrf_guard(
    state: &AdminState,
    session: &AuthenticatedSession,
    csrf_token: &str,
) -> Option<Response> {
    if state
        .sessions
        .validate_csrf(&session.token, csrf_token)
        .await
    {
        None
    } else {
        Some((StatusCode::FORBIDDEN, "invalid CSRF token").into_response())
    }
}

fn capability_guard(
    headers: &HeaderMap,
    session: &AuthenticatedSession,
    capability: AdminCapability,
) -> Option<Response> {
    if session.must_change_password
        && !matches!(
            capability,
            AdminCapability::InventoryRead | AdminCapability::SessionsRead
        )
    {
        return Some(Redirect::to("/settings").into_response());
    }
    if profile_has_capability(&session.access_profile, capability) {
        return None;
    }
    let t = request_l10n(headers);
    let task = match capability {
        AdminCapability::InventoryRead | AdminCapability::AssetsManage => t.nav_assets,
        AdminCapability::CredentialsManage => t.nav_credentials,
        AdminCapability::AccessManage => t.nav_keys,
        AdminCapability::SessionsRead => t.nav_sessions,
        AdminCapability::AdminsManage => t.nav_settings,
    };
    Some(
        (
            StatusCode::FORBIDDEN,
            Html(html::permission_denied(t, task, &session.access_profile).into_string()),
        )
            .into_response(),
    )
}

type OptionalPeer = Option<Extension<ConnectInfo<SocketAddr>>>;

fn source_ip(peer: Option<&Extension<ConnectInfo<SocketAddr>>>) -> Option<String> {
    peer.map(|Extension(ConnectInfo(address))| address.ip().to_string())
}

#[allow(clippy::too_many_arguments)]
async fn record_audit(
    state: &AdminState,
    session: Option<&AuthenticatedSession>,
    source_ip: Option<String>,
    action: &str,
    target_type: &str,
    target_id: Option<&str>,
    target_label: Option<&str>,
    result: &str,
    details: Option<Value>,
) {
    let (actor_id, actor_label) = match session {
        Some(session) => (Some(session.admin_id.clone()), session.admin_label.clone()),
        None => (None, "Unauthenticated".to_string()),
    };
    let details_json = details.and_then(|details| serde_json::to_string(&details).ok());
    if let Err(err) = state
        .db
        .add_audit_event(NewAuditEvent {
            actor_id,
            actor_label,
            action: action.to_string(),
            target_type: target_type.to_string(),
            target_id: target_id.map(ToString::to_string),
            target_label: target_label.map(ToString::to_string),
            result: result.to_string(),
            source_ip,
            details_json,
        })
        .await
    {
        warn!(%err, %action, "failed to persist audit event");
    }
}

async fn index(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::InventoryRead) {
        return resp;
    }
    let database_healthy = state.db.health_check().await.is_ok();
    let ssh_reachable = probe_tcp_bind(state.ssh_bind).await;
    let mut source_errors = Vec::new();
    let assets = dashboard_value(state.db.list_assets().await, "assets", &mut source_errors);
    let credentials = dashboard_value(
        state.db.list_credentials().await,
        "credentials",
        &mut source_errors,
    );
    let keys = dashboard_value(
        state.db.list_authorized_keys().await,
        "ssh identities",
        &mut source_errors,
    );
    let known_hosts = dashboard_value(
        state.db.list_known_hosts().await,
        "known hosts",
        &mut source_errors,
    );
    let asset_health = dashboard_value(
        state.db.list_asset_health().await,
        "asset health",
        &mut source_errors,
    );
    let recent_sessions = dashboard_value(
        state.db.list_sessions(8).await,
        "recent sessions",
        &mut source_errors,
    );
    let sessions_24h = dashboard_value(
        state.db.count_sessions_since_hours(24).await,
        "24 hour session count",
        &mut source_errors,
    );
    let recent_admin_events = dashboard_value(
        state.db.list_audit_events(6).await,
        "admin activity",
        &mut source_errors,
    );
    Html(
        html::overview(
            t,
            &html::DashboardData {
                gateway: html::DashboardGateway {
                    admin_bind: state.admin_bind.to_string(),
                    ssh_bind: state.ssh_bind.to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    started_at: state.started_at.clone(),
                    uptime_seconds: state.started_instant.elapsed().as_secs(),
                    admin_reachable: true,
                    ssh_reachable,
                    database_healthy,
                },
                assets,
                credentials,
                keys,
                known_hosts,
                asset_health,
                recent_sessions,
                sessions_24h,
                recent_admin_events,
                source_errors,
            },
        )
        .into_string(),
    )
    .into_response()
}

fn dashboard_value<T: Default, E: std::fmt::Display>(
    result: std::result::Result<T, E>,
    source: &str,
    errors: &mut Vec<String>,
) -> T {
    match result {
        Ok(value) => value,
        Err(err) => {
            warn!(%source, error = %err, "dashboard data source failed");
            errors.push(source.to_string());
            T::default()
        }
    }
}

fn probe_address(bind: SocketAddr) -> SocketAddr {
    let ip = match bind.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, bind.port())
}

async fn probe_tcp_bind(bind: SocketAddr) -> bool {
    matches!(
        timeout(
            Duration::from_millis(350),
            TcpStream::connect(probe_address(bind))
        )
        .await,
        Ok(Ok(_))
    )
}

async fn login_page(State(state): State<AdminState>, headers: HeaderMap) -> Html<String> {
    let t = request_l10n(&headers);
    let show_username = state
        .db
        .count_active_admin_users()
        .await
        .map(|count| count > 1)
        .unwrap_or(true);
    Html(html::login(t, None, show_username, None).into_string())
}

#[derive(Deserialize)]
struct LoginForm {
    username: Option<String>,
    password: String,
}

async fn login(
    State(state): State<AdminState>,
    headers: HeaderMap,
    peer: OptionalPeer,
    Form(form): Form<LoginForm>,
) -> Response {
    let t = request_l10n(&headers);
    let active_admins = state
        .db
        .list_admin_users()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|admin| admin.is_active)
        .collect::<Vec<_>>();
    let show_username = active_admins.len() > 1;
    let submitted_username = form
        .username
        .as_deref()
        .map(str::trim)
        .filter(|username| !username.is_empty());
    let admin = if show_username {
        submitted_username.and_then(|username| {
            active_admins
                .iter()
                .find(|admin| admin.username.eq_ignore_ascii_case(username))
                .cloned()
        })
    } else {
        active_admins.into_iter().next()
    };
    let verified = match &admin {
        Some(admin) => bootstrap::verify_admin_user_password(&state.db, &admin.id, &form.password)
            .await
            .unwrap_or(false),
        None => false,
    };
    match (verified, admin) {
        (true, Some(admin)) => {
            let _ = state.db.mark_admin_login(&admin.id).await;
            let token = state
                .sessions
                .create(
                    &admin.id,
                    &admin.display_name,
                    &admin.access_profile,
                    admin.must_change_password,
                )
                .await;
            let authenticated = state.sessions.authenticate(&token).await;
            record_audit(
                &state,
                authenticated.as_ref(),
                source_ip(peer.as_ref()),
                "admin.login",
                "admin_user",
                Some(&admin.id),
                Some(&admin.display_name),
                "success",
                None,
            )
            .await;
            (
                StatusCode::SEE_OTHER,
                [
                    (
                        header::SET_COOKIE,
                        session_cookie(&token, state.cookie_secure),
                    ),
                    (
                        header::LOCATION,
                        if admin.must_change_password {
                            "/settings".to_string()
                        } else {
                            "/".to_string()
                        },
                    ),
                ],
            )
                .into_response()
        }
        _ => {
            record_audit(
                &state,
                None,
                source_ip(peer.as_ref()),
                "admin.login",
                "admin_user",
                None,
                Some(submitted_username.unwrap_or("password-only")),
                "failure",
                None,
            )
            .await;
            Html(
                html::login(
                    t,
                    Some(t.login_invalid_password),
                    show_username,
                    submitted_username,
                )
                .into_string(),
            )
            .into_response()
        }
    }
}

async fn logout(
    State(state): State<AdminState>,
    headers: HeaderMap,
    peer: OptionalPeer,
) -> Response {
    if let Some(token) = cookie_token(&headers) {
        let session = state.sessions.authenticate(&token).await;
        record_audit(
            &state,
            session.as_ref(),
            source_ip(peer.as_ref()),
            "admin.logout",
            "admin_session",
            None,
            None,
            "success",
            None,
        )
        .await;
        state.sessions.remove(&token).await;
    }
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, clear_cookie(state.cookie_secure)),
            (header::LOCATION, "/login".to_string()),
        ],
    )
        .into_response()
}

async fn settings(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    render_settings(&state, t, &session, None).await
}

async fn render_settings(
    state: &AdminState,
    t: &L10n,
    session: &AuthenticatedSession,
    error: Option<&str>,
) -> Response {
    let current_admin = match state.db.get_admin_user_by_id(&session.admin_id).await {
        Ok(Some(admin)) => admin,
        Ok(None) => return Redirect::to("/login").into_response(),
        Err(err) => return admin_db_error("load current admin settings", err),
    };
    let admins = match state.db.list_admin_users().await {
        Ok(admins) => admins,
        Err(err) => return admin_db_error("list admins for settings", err),
    };
    Html(
        html::settings(
            t,
            &current_admin,
            &admins,
            &session.csrf_token,
            error,
            profile_has_capability(&session.access_profile, AdminCapability::AdminsManage),
        )
        .into_string(),
    )
    .into_response()
}

#[derive(Deserialize)]
struct SettingsForm {
    csrf_token: String,
    current_password: String,
    new_password: String,
    confirm_password: String,
}

async fn update_settings(
    State(state): State<AdminState>,
    headers: HeaderMap,
    peer: OptionalPeer,
    Form(form): Form<SettingsForm>,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    match bootstrap::change_admin_user_password(
        &state.db,
        &session.admin_id,
        &form.current_password,
        &form.new_password,
        &form.confirm_password,
    )
    .await
    {
        Ok(Ok(())) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "admin.password_change",
                "admin_user",
                Some(&session.admin_id),
                Some(&session.admin_label),
                "success",
                None,
            )
            .await;
            state.sessions.remove(&session.token).await;
            (
                StatusCode::SEE_OTHER,
                [
                    (header::SET_COOKIE, clear_cookie(state.cookie_secure)),
                    (header::LOCATION, "/login".to_string()),
                ],
            )
                .into_response()
        }
        Ok(Err(err)) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "admin.password_change",
                "admin_user",
                Some(&session.admin_id),
                Some(&session.admin_label),
                "failure",
                Some(serde_json::json!({
                    "reason": settings_password_error_code(err)
                })),
            )
            .await;
            render_settings(
                &state,
                t,
                &session,
                Some(settings_password_error_message(t, err)),
            )
            .await
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to change admin password",
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct CreateAdminForm {
    csrf_token: String,
    display_name: String,
    username: String,
    temporary_password: String,
    access_profile: String,
}

async fn create_admin_user(
    State(state): State<AdminState>,
    headers: HeaderMap,
    peer: OptionalPeer,
    Form(form): Form<CreateAdminForm>,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AdminsManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    if form.temporary_password.chars().count() < 12 {
        return render_settings(
            &state,
            t,
            &session,
            Some(t.admin_temporary_password_too_short),
        )
        .await;
    }
    let password_hash = match bootstrap::hash_password(&form.temporary_password) {
        Ok(hash) => hash,
        Err(err) => return admin_db_error("hash temporary admin password", err),
    };
    match state
        .db
        .add_admin_user(NewAdminUser {
            username: form.username,
            display_name: form.display_name,
            password_hash,
            access_profile: form.access_profile,
            must_change_password: true,
        })
        .await
    {
        Ok(admin) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "admin_user.create",
                "admin_user",
                Some(&admin.id),
                Some(&admin.display_name),
                "success",
                Some(serde_json::json!({
                    "access_profile": admin.access_profile,
                    "must_change_password": true
                })),
            )
            .await;
            Redirect::to("/settings").into_response()
        }
        Err(err) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "admin_user.create",
                "admin_user",
                None,
                None,
                "failure",
                Some(serde_json::json!({"reason": "validation_or_database"})),
            )
            .await;
            render_settings(&state, t, &session, Some(&err.to_string())).await
        }
    }
}

#[derive(Deserialize)]
struct UpdateAdminAccessForm {
    csrf_token: String,
    access_profile: String,
    is_active: Option<String>,
}

async fn update_admin_user_access(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    peer: OptionalPeer,
    Form(form): Form<UpdateAdminAccessForm>,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AdminsManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let before = match state.db.get_admin_user_by_id(&id).await {
        Ok(Some(admin)) => admin,
        Ok(None) => return render_settings(&state, t, &session, Some(t.admin_not_found)).await,
        Err(err) => return admin_db_error("load admin before access update", err),
    };
    let is_active = form.is_active.as_deref() == Some("yes");
    match state
        .db
        .update_admin_user_access(&id, &form.access_profile, is_active)
        .await
    {
        Ok(()) => {
            state.sessions.remove_admin(&id).await;
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "admin_user.access_update",
                "admin_user",
                Some(&id),
                Some(&before.display_name),
                "success",
                Some(serde_json::json!({
                    "before": {
                        "access_profile": before.access_profile,
                        "is_active": before.is_active
                    },
                    "after": {
                        "access_profile": form.access_profile,
                        "is_active": is_active
                    }
                })),
            )
            .await;
            if id == session.admin_id {
                (
                    StatusCode::SEE_OTHER,
                    [
                        (header::SET_COOKIE, clear_cookie(state.cookie_secure)),
                        (header::LOCATION, "/login".to_string()),
                    ],
                )
                    .into_response()
            } else {
                Redirect::to("/settings").into_response()
            }
        }
        Err(err) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "admin_user.access_update",
                "admin_user",
                Some(&id),
                Some(&before.display_name),
                "failure",
                Some(serde_json::json!({"reason": "guard_or_database"})),
            )
            .await;
            let error = err.to_string();
            let message = if error.contains("last full-control admin") {
                t.admin_last_owner_note
            } else {
                &error
            };
            render_settings(&state, t, &session, Some(message)).await
        }
    }
}

#[derive(Deserialize)]
struct AssetsQuery {
    tag: Option<String>,
    q: Option<String>,
}

async fn assets(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Query(query): Query<AssetsQuery>,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let all_assets = match state.db.list_assets().await {
        Ok(assets) => assets,
        Err(err) => return admin_db_error("list assets", err),
    };
    let all_tags = collect_tags(&all_assets);
    let assets = filter_assets(&all_assets, query.tag.as_deref(), query.q.as_deref());
    let credentials = match state.db.list_credentials().await {
        Ok(credentials) => credentials,
        Err(err) => return admin_db_error("list credentials for assets", err),
    };
    Html(
        html::assets(
            t,
            &assets,
            &credentials,
            &session.csrf_token,
            query.tag.as_deref(),
            query.q.as_deref(),
            &all_tags,
            state.ssh_port,
            profile_has_capability(&session.access_profile, AdminCapability::AssetsManage),
        )
        .into_string(),
    )
    .into_response()
}

#[derive(Deserialize)]
struct CsrfForm {
    csrf_token: String,
}

#[derive(Deserialize)]
struct AssetForm {
    csrf_token: String,
    return_to: Option<String>,
    name: String,
    protocol: Option<String>,
    preset: Option<String>,
    hostname: String,
    port: i64,
    description: Option<String>,
    tags: Option<String>,
    credential_id: Option<String>,
}

#[derive(Deserialize)]
struct BulkTagsForm {
    csrf_token: String,
    #[serde(default)]
    asset_ids: Vec<String>,
    tags: Option<String>,
}

async fn create_asset(
    State(state): State<AdminState>,
    headers: HeaderMap,
    peer: OptionalPeer,
    Form(form): Form<AssetForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AssetsManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let return_to = assets_return_to(form.return_to.as_deref());
    let requested_name = form.name.clone();
    let Some(asset) = new_asset_from_form(form) else {
        record_audit(
            &state,
            Some(&session),
            source_ip(peer.as_ref()),
            "asset.create",
            "asset",
            None,
            Some(&requested_name),
            "failure",
            Some(serde_json::json!({"reason": "validation"})),
        )
        .await;
        return Redirect::to(&return_to).into_response();
    };
    match state.db.add_asset(asset).await {
        Ok(created) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "asset.create",
                "asset",
                Some(&created.id),
                Some(&created.name),
                "success",
                Some(serde_json::json!({
                    "protocol": created.protocol,
                    "preset": created.preset,
                    "hostname": created.hostname,
                    "port": created.port
                })),
            )
            .await;
            Redirect::to(&return_to).into_response()
        }
        Err(err) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "asset.create",
                "asset",
                None,
                Some(&requested_name),
                "failure",
                Some(serde_json::json!({"reason": "database"})),
            )
            .await;
            admin_db_error("create asset", err)
        }
    }
}

#[derive(Default, Deserialize)]
struct EditAssetQuery {
    return_to: Option<String>,
}

async fn edit_asset(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<EditAssetQuery>,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AssetsManage) {
        return resp;
    }
    let Ok(Some(asset)) = state.db.get_asset_by_id(&id).await else {
        return Redirect::to("/assets").into_response();
    };
    let credentials = match state.db.list_credentials().await {
        Ok(credentials) => credentials,
        Err(err) => return admin_db_error("list credentials for asset edit", err),
    };
    let all_assets = match state.db.list_assets().await {
        Ok(assets) => assets,
        Err(err) => return admin_db_error("list assets for asset edit", err),
    };
    let all_tags = collect_tags(&all_assets);
    let return_to = assets_return_to(query.return_to.as_deref());
    Html(
        html::edit_asset(
            t,
            &asset,
            &credentials,
            &session.csrf_token,
            &all_tags,
            state.ssh_port,
            &return_to,
        )
        .into_string(),
    )
    .into_response()
}

async fn update_asset(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    peer: OptionalPeer,
    Form(form): Form<AssetForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AssetsManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let return_to = assets_return_to(form.return_to.as_deref());
    let requested_name = form.name.clone();
    let Some(asset) = new_asset_from_form(form) else {
        record_audit(
            &state,
            Some(&session),
            source_ip(peer.as_ref()),
            "asset.update",
            "asset",
            Some(&id),
            Some(&requested_name),
            "failure",
            Some(serde_json::json!({"reason": "validation"})),
        )
        .await;
        return Redirect::to(&return_to).into_response();
    };
    let audit_asset = asset.clone();
    match state.db.update_asset(&id, asset).await {
        Ok(()) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "asset.update",
                "asset",
                Some(&id),
                Some(&audit_asset.name),
                "success",
                Some(serde_json::json!({
                    "protocol": audit_asset.protocol,
                    "preset": audit_asset.preset,
                    "hostname": audit_asset.hostname,
                    "port": audit_asset.port
                })),
            )
            .await;
            Redirect::to(&return_to).into_response()
        }
        Err(err) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "asset.update",
                "asset",
                Some(&id),
                Some(&requested_name),
                "failure",
                Some(serde_json::json!({"reason": "database"})),
            )
            .await;
            admin_db_error("update asset", err)
        }
    }
}

async fn delete_asset(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    peer: OptionalPeer,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AssetsManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let target_label = state
        .db
        .get_asset_by_id(&id)
        .await
        .ok()
        .flatten()
        .map(|asset| asset.name);
    match state.db.delete_asset(&id).await {
        Ok(()) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "asset.delete",
                "asset",
                Some(&id),
                target_label.as_deref(),
                "success",
                None,
            )
            .await;
            Redirect::to("/assets").into_response()
        }
        Err(err) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "asset.delete",
                "asset",
                Some(&id),
                target_label.as_deref(),
                "failure",
                Some(serde_json::json!({"reason": "database"})),
            )
            .await;
            admin_db_error("delete asset", err)
        }
    }
}

async fn bulk_update_asset_tags(
    State(state): State<AdminState>,
    headers: HeaderMap,
    peer: OptionalPeer,
    body: Bytes,
) -> Response {
    let Ok(form) = parse_bulk_tags_body(&body) else {
        return (StatusCode::BAD_REQUEST, "invalid bulk tag form").into_response();
    };
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AssetsManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let tags = parse_tags(form.tags);
    let asset_count = form.asset_ids.len();
    for asset_id in form.asset_ids {
        if let Ok(Some(asset)) = state.db.get_asset_by_id(&asset_id).await {
            if let Err(err) = state
                .db
                .update_asset(
                    &asset.id,
                    NewAsset {
                        name: asset.name,
                        protocol: asset.protocol,
                        preset: asset.preset,
                        hostname: asset.hostname,
                        port: asset.port,
                        description: asset.description,
                        tags: tags.clone(),
                        credential_id: asset.credential_id,
                    },
                )
                .await
            {
                return admin_db_error("bulk update asset tags", err);
            }
        }
    }
    record_audit(
        &state,
        Some(&session),
        source_ip(peer.as_ref()),
        "asset.bulk_tags",
        "asset_batch",
        None,
        None,
        "success",
        Some(serde_json::json!({"asset_count": asset_count})),
    )
    .await;
    Redirect::to("/assets").into_response()
}

async fn credentials(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let credentials = match state.db.list_credentials().await {
        Ok(credentials) => credentials,
        Err(err) => return admin_db_error("list credentials", err),
    };
    let assets = match state.db.list_assets().await {
        Ok(assets) => assets,
        Err(err) => return admin_db_error("list assets for credential usage", err),
    };
    Html(
        html::credentials(
            t,
            &credentials,
            &assets,
            &session.csrf_token,
            None,
            profile_has_capability(&session.access_profile, AdminCapability::CredentialsManage),
        )
        .into_string(),
    )
    .into_response()
}

#[derive(Deserialize)]
struct CredentialForm {
    csrf_token: String,
    name: String,
    username: String,
    auth_type: String,
    password: Option<String>,
    private_key: Option<String>,
    passphrase: Option<String>,
}

async fn create_credential(
    State(state): State<AdminState>,
    headers: HeaderMap,
    peer: OptionalPeer,
    Form(form): Form<CredentialForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::CredentialsManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let requested_name = form.name.clone();
    let requested_auth_type = form.auth_type.clone();
    let Ok(auth_type) = AuthType::try_from(form.auth_type.as_str()) else {
        record_audit(
            &state,
            Some(&session),
            source_ip(peer.as_ref()),
            "credential.create",
            "credential",
            None,
            Some(&requested_name),
            "failure",
            Some(serde_json::json!({"reason": "invalid_auth_type"})),
        )
        .await;
        return Redirect::to("/credentials").into_response();
    };
    let id = new_id();
    let password_enc = encrypt_optional(&state.master_key, &id, "password", form.password)
        .ok()
        .flatten();
    let private_key_enc = encrypt_optional(&state.master_key, &id, "private_key", form.private_key)
        .ok()
        .flatten();
    let passphrase_enc = encrypt_optional(&state.master_key, &id, "passphrase", form.passphrase)
        .ok()
        .flatten();
    if validate_credential_material(
        &auth_type,
        password_enc.as_deref(),
        private_key_enc.as_deref(),
        passphrase_enc.as_deref(),
    )
    .is_err()
    {
        record_audit(
            &state,
            Some(&session),
            source_ip(peer.as_ref()),
            "credential.create",
            "credential",
            None,
            Some(&requested_name),
            "failure",
            Some(serde_json::json!({"reason": "invalid_secret_material"})),
        )
        .await;
        return Redirect::to("/credentials").into_response();
    }
    match state
        .db
        .add_credential(NewCredential {
            id: Some(id),
            name: form.name,
            username: form.username,
            auth_type,
            password_enc,
            private_key_enc,
            passphrase_enc,
        })
        .await
    {
        Ok(created) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "credential.create",
                "credential",
                Some(&created.id),
                Some(&created.name),
                "success",
                Some(serde_json::json!({"auth_type": created.auth_type})),
            )
            .await;
            Redirect::to("/credentials").into_response()
        }
        Err(err) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "credential.create",
                "credential",
                None,
                Some(&requested_name),
                "failure",
                Some(serde_json::json!({
                    "reason": "database",
                    "auth_type": requested_auth_type
                })),
            )
            .await;
            admin_db_error("create credential", err)
        }
    }
}

async fn edit_credential(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::CredentialsManage) {
        return resp;
    }
    let Ok(Some(credential)) = state.db.get_credential(&id).await else {
        return Redirect::to("/credentials").into_response();
    };
    let assets = match state.db.list_assets().await {
        Ok(assets) => assets,
        Err(err) => return admin_db_error("list assets for credential edit", err),
    };
    Html(html::edit_credential(t, &credential, &assets, &session.csrf_token).into_string())
        .into_response()
}

async fn update_credential(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    peer: OptionalPeer,
    Form(form): Form<CredentialForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::CredentialsManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let Ok(Some(existing)) = state.db.get_credential(&id).await else {
        return Redirect::to("/credentials").into_response();
    };
    let requested_name = form.name.clone();
    let requested_auth_type = form.auth_type.clone();
    let Ok(auth_type) = AuthType::try_from(form.auth_type.as_str()) else {
        record_audit(
            &state,
            Some(&session),
            source_ip(peer.as_ref()),
            "credential.update",
            "credential",
            Some(&id),
            Some(&requested_name),
            "failure",
            Some(serde_json::json!({"reason": "invalid_auth_type"})),
        )
        .await;
        return Redirect::to("/credentials").into_response();
    };
    let password_enc = encrypt_optional(&state.master_key, &id, "password", form.password)
        .ok()
        .flatten()
        .or(existing.password_enc);
    let private_key_enc = encrypt_optional(&state.master_key, &id, "private_key", form.private_key)
        .ok()
        .flatten()
        .or(existing.private_key_enc);
    let passphrase_enc = encrypt_optional(&state.master_key, &id, "passphrase", form.passphrase)
        .ok()
        .flatten()
        .or(existing.passphrase_enc);
    if validate_credential_material(
        &auth_type,
        password_enc.as_deref(),
        private_key_enc.as_deref(),
        passphrase_enc.as_deref(),
    )
    .is_err()
    {
        record_audit(
            &state,
            Some(&session),
            source_ip(peer.as_ref()),
            "credential.update",
            "credential",
            Some(&id),
            Some(&requested_name),
            "failure",
            Some(serde_json::json!({"reason": "invalid_secret_material"})),
        )
        .await;
        return Redirect::to("/credentials").into_response();
    }
    match state
        .db
        .update_credential(
            &id,
            NewCredential {
                id: Some(id.clone()),
                name: form.name,
                username: form.username,
                auth_type,
                password_enc,
                private_key_enc,
                passphrase_enc,
            },
        )
        .await
    {
        Ok(()) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "credential.update",
                "credential",
                Some(&id),
                Some(&requested_name),
                "success",
                Some(serde_json::json!({"auth_type": requested_auth_type})),
            )
            .await;
            Redirect::to("/credentials").into_response()
        }
        Err(err) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "credential.update",
                "credential",
                Some(&id),
                Some(&requested_name),
                "failure",
                Some(serde_json::json!({"reason": "database"})),
            )
            .await;
            admin_db_error("update credential", err)
        }
    }
}

fn encrypt_optional(
    master_key: &MasterKey,
    id: &str,
    field: &str,
    value: Option<String>,
) -> anyhow::Result<Option<String>> {
    match value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(value) => Ok(Some(encrypt_envelope(
            master_key,
            &format!("{id}:{field}"),
            value.as_bytes(),
        )?)),
        None => Ok(None),
    }
}

fn parse_tags(tags: Option<String>) -> Vec<String> {
    tags.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn new_asset_from_form(form: AssetForm) -> Option<NewAsset> {
    let (protocol, preset) = hop_core::normalize_asset_protocol(
        form.protocol.as_deref().unwrap_or(ASSET_PROTOCOL_SSH),
        form.preset.as_deref(),
    )
    .ok()?;
    validate_tcp_port(form.port).ok()?;
    let credential_id = if protocol_supports_managed_credentials(&protocol) {
        form.credential_id.filter(|value| !value.trim().is_empty())
    } else {
        None
    };
    Some(NewAsset {
        name: form.name,
        protocol,
        preset,
        hostname: form.hostname,
        port: form.port,
        description: form.description.filter(|v| !v.trim().is_empty()),
        tags: parse_tags(form.tags),
        credential_id,
    })
}

async fn delete_credential(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    peer: OptionalPeer,
    Form(form): Form<CsrfForm>,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::CredentialsManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let assets = match state.db.list_assets().await {
        Ok(assets) => assets,
        Err(err) => return admin_db_error("list assets before deleting credential", err),
    };
    let target_label = state
        .db
        .get_credential(&id)
        .await
        .ok()
        .flatten()
        .map(|credential| credential.name);
    if credential_is_in_use(&assets, &id) {
        record_audit(
            &state,
            Some(&session),
            source_ip(peer.as_ref()),
            "credential.delete",
            "credential",
            Some(&id),
            target_label.as_deref(),
            "failure",
            Some(serde_json::json!({"reason": "in_use"})),
        )
        .await;
        let credentials = match state.db.list_credentials().await {
            Ok(credentials) => credentials,
            Err(err) => return admin_db_error("list credentials after delete conflict", err),
        };
        return (
            StatusCode::CONFLICT,
            Html(
                html::credentials(
                    t,
                    &credentials,
                    &assets,
                    &session.csrf_token,
                    Some(t.credential_delete_in_use),
                    true,
                )
                .into_string(),
            ),
        )
            .into_response();
    }
    match state.db.delete_credential(&id).await {
        Ok(()) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "credential.delete",
                "credential",
                Some(&id),
                target_label.as_deref(),
                "success",
                None,
            )
            .await;
            Redirect::to("/credentials").into_response()
        }
        Err(err) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "credential.delete",
                "credential",
                Some(&id),
                target_label.as_deref(),
                "failure",
                Some(serde_json::json!({"reason": "database"})),
            )
            .await;
            admin_db_error("delete credential", err)
        }
    }
}

async fn keys(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let keys = match state.db.list_authorized_keys().await {
        Ok(keys) => keys,
        Err(err) => return admin_db_error("list authorized keys", err),
    };
    let assets = match state.db.list_assets().await {
        Ok(assets) => assets,
        Err(err) => return admin_db_error("list assets for authorized keys", err),
    };
    Html(
        html::keys(
            t,
            &keys,
            &assets,
            &session.csrf_token,
            None,
            profile_has_capability(&session.access_profile, AdminCapability::AccessManage),
        )
        .into_string(),
    )
    .into_response()
}

#[derive(Debug, PartialEq, Eq)]
struct KeyAccessForm {
    csrf_token: String,
    name: String,
    public_key: String,
    asset_access_mode: String,
    asset_ids: Vec<String>,
}

async fn create_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    peer: OptionalPeer,
    body: Bytes,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AccessManage) {
        return resp;
    }
    let form = match parse_key_access_form(&body) {
        Ok(form) => form,
        Err(err) => return render_keys_error(&state, t, &session, &err.to_string()).await,
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let mode = match AssetAccessMode::try_from(form.asset_access_mode.as_str()) {
        Ok(mode) => mode,
        Err(err) => return render_keys_error(&state, t, &session, &err.to_string()).await,
    };
    let requested_name = form.name.clone();
    let (public_key, fingerprint) = match parse_public_key_line(&form.public_key) {
        Ok(parsed) => parsed,
        Err(err) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "ssh_identity.create",
                "authorized_key",
                None,
                Some(&requested_name),
                "failure",
                Some(serde_json::json!({"reason": "invalid_public_key"})),
            )
            .await;
            return render_keys_error(&state, t, &session, &err.to_string()).await;
        }
    };
    let audit_mode = form.asset_access_mode.clone();
    let audit_asset_count = form.asset_ids.len();
    match state
        .db
        .add_authorized_key_with_access(
            NewAuthorizedKey::new(form.name, public_key, fingerprint),
            mode,
            &form.asset_ids,
        )
        .await
    {
        Ok(created) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "ssh_identity.create",
                "authorized_key",
                Some(&created.id),
                Some(&created.name),
                "success",
                Some(serde_json::json!({
                    "fingerprint": created.fingerprint,
                    "access_mode": audit_mode,
                    "asset_count": audit_asset_count
                })),
            )
            .await;
            Redirect::to("/keys").into_response()
        }
        Err(err) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "ssh_identity.create",
                "authorized_key",
                None,
                Some(&requested_name),
                "failure",
                Some(serde_json::json!({"reason": "database"})),
            )
            .await;
            render_keys_error(&state, t, &session, &err.to_string()).await
        }
    }
}

async fn render_keys_error(
    state: &AdminState,
    t: &L10n,
    session: &AuthenticatedSession,
    error: &str,
) -> Response {
    let keys = match state.db.list_authorized_keys().await {
        Ok(keys) => keys,
        Err(err) => return admin_db_error("list authorized keys after create error", err),
    };
    let assets = match state.db.list_assets().await {
        Ok(assets) => assets,
        Err(err) => return admin_db_error("list assets after key create error", err),
    };
    (
        StatusCode::BAD_REQUEST,
        Html(html::keys(t, &keys, &assets, &session.csrf_token, Some(error), true).into_string()),
    )
        .into_response()
}

async fn edit_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AccessManage) {
        return resp;
    }
    let Ok(Some(key)) = state.db.get_authorized_key_by_id(&id).await else {
        return Redirect::to("/keys").into_response();
    };
    let assets = match state.db.list_assets().await {
        Ok(assets) => assets,
        Err(err) => return admin_db_error("list assets for key edit", err),
    };
    let assigned_ids = match state.db.list_asset_ids_for_key(&id).await {
        Ok(ids) => ids,
        Err(err) => return admin_db_error("list key asset assignments", err),
    };
    Html(html::edit_key(t, &key, &assets, &assigned_ids, &session.csrf_token, None).into_string())
        .into_response()
}

async fn update_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    peer: OptionalPeer,
    body: Bytes,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AccessManage) {
        return resp;
    }
    let form = match parse_key_access_form(&body) {
        Ok(form) => form,
        Err(err) => {
            return render_key_edit_error(&state, t, &session, &id, &err.to_string()).await;
        }
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let mode = match AssetAccessMode::try_from(form.asset_access_mode.as_str()) {
        Ok(mode) => mode,
        Err(err) => {
            return render_key_edit_error(&state, t, &session, &id, &err.to_string()).await;
        }
    };
    let (public_key, fingerprint) = match parse_public_key_line(&form.public_key) {
        Ok(parsed) => parsed,
        Err(err) => {
            return render_key_edit_error(&state, t, &session, &id, &err.to_string()).await;
        }
    };
    let audit_name = form.name.clone();
    let audit_mode = form.asset_access_mode.clone();
    let audit_asset_count = form.asset_ids.len();
    let audit_fingerprint = fingerprint.clone();
    match state
        .db
        .update_authorized_key_with_access(
            &id,
            NewAuthorizedKey::new(form.name, public_key, fingerprint),
            mode,
            &form.asset_ids,
        )
        .await
    {
        Ok(()) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "ssh_identity.update",
                "authorized_key",
                Some(&id),
                Some(&audit_name),
                "success",
                Some(serde_json::json!({
                    "access_mode": audit_mode,
                    "asset_count": audit_asset_count,
                    "fingerprint": audit_fingerprint
                })),
            )
            .await;
            Redirect::to("/keys").into_response()
        }
        Err(err) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "ssh_identity.update",
                "authorized_key",
                Some(&id),
                Some(&audit_name),
                "failure",
                Some(serde_json::json!({"reason": "database"})),
            )
            .await;
            render_key_edit_error(&state, t, &session, &id, &err.to_string()).await
        }
    }
}

fn parse_key_access_form(body: &[u8]) -> Result<KeyAccessForm> {
    let mut form = KeyAccessForm {
        csrf_token: String::new(),
        name: String::new(),
        public_key: String::new(),
        asset_access_mode: String::new(),
        asset_ids: Vec::new(),
    };
    for (key, value) in form_urlencoded::parse(body) {
        match key.as_ref() {
            "csrf_token" => form.csrf_token = value.into_owned(),
            "name" => form.name = value.into_owned(),
            "public_key" => form.public_key = value.into_owned(),
            "asset_access_mode" => form.asset_access_mode = value.into_owned(),
            "asset_id" => form.asset_ids.push(value.into_owned()),
            _ => {}
        }
    }
    ensure!(!form.csrf_token.is_empty(), "missing CSRF token");
    ensure!(!form.name.trim().is_empty(), "key name is required");
    ensure!(!form.public_key.trim().is_empty(), "public key is required");
    ensure!(
        !form.asset_access_mode.is_empty(),
        "asset access mode is required"
    );
    Ok(form)
}

async fn render_key_edit_error(
    state: &AdminState,
    t: &L10n,
    session: &AuthenticatedSession,
    id: &str,
    error: &str,
) -> Response {
    let Ok(Some(key)) = state.db.get_authorized_key_by_id(id).await else {
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    };
    let assets = match state.db.list_assets().await {
        Ok(assets) => assets,
        Err(err) => return admin_db_error("list assets for key edit error", err),
    };
    let assigned_ids = match state.db.list_asset_ids_for_key(id).await {
        Ok(ids) => ids,
        Err(err) => return admin_db_error("list key assignments for key edit error", err),
    };
    (
        StatusCode::BAD_REQUEST,
        Html(
            html::edit_key(
                t,
                &key,
                &assets,
                &assigned_ids,
                &session.csrf_token,
                Some(error),
            )
            .into_string(),
        ),
    )
        .into_response()
}

async fn deactivate_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    peer: OptionalPeer,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AccessManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    set_key_active(&state, &session, peer.as_ref(), &id, false).await
}

async fn activate_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    peer: OptionalPeer,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AccessManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    set_key_active(&state, &session, peer.as_ref(), &id, true).await
}

async fn set_key_active(
    state: &AdminState,
    session: &AuthenticatedSession,
    peer: Option<&Extension<ConnectInfo<SocketAddr>>>,
    id: &str,
    active: bool,
) -> Response {
    let target_label = state
        .db
        .get_authorized_key_by_id(id)
        .await
        .ok()
        .flatten()
        .map(|key| key.name);
    let action = if active {
        "ssh_identity.activate"
    } else {
        "ssh_identity.deactivate"
    };
    match state.db.set_authorized_key_active(id, active).await {
        Ok(()) => {
            record_audit(
                state,
                Some(session),
                source_ip(peer),
                action,
                "authorized_key",
                Some(id),
                target_label.as_deref(),
                "success",
                None,
            )
            .await;
            Redirect::to("/keys").into_response()
        }
        Err(err) => {
            record_audit(
                state,
                Some(session),
                source_ip(peer),
                action,
                "authorized_key",
                Some(id),
                target_label.as_deref(),
                "failure",
                Some(serde_json::json!({"reason": "database"})),
            )
            .await;
            admin_db_error("change authorized key state", err)
        }
    }
}

async fn delete_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    peer: OptionalPeer,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AccessManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let target_label = state
        .db
        .get_authorized_key_by_id(&id)
        .await
        .ok()
        .flatten()
        .map(|key| key.name);
    match state.db.delete_authorized_key(&id).await {
        Ok(()) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "ssh_identity.delete",
                "authorized_key",
                Some(&id),
                target_label.as_deref(),
                "success",
                None,
            )
            .await;
            Redirect::to("/keys").into_response()
        }
        Err(err) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "ssh_identity.delete",
                "authorized_key",
                Some(&id),
                target_label.as_deref(),
                "failure",
                Some(serde_json::json!({"reason": "database"})),
            )
            .await;
            admin_db_error("delete authorized key", err)
        }
    }
}

async fn known_hosts(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let hosts = match state.db.list_known_hosts().await {
        Ok(hosts) => hosts,
        Err(err) => return admin_db_error("list known hosts", err),
    };
    let assets = match state.db.list_assets().await {
        Ok(assets) => assets,
        Err(err) => return admin_db_error("list assets for known hosts", err),
    };
    Html(
        html::known_hosts(
            t,
            &hosts,
            &assets,
            &session.csrf_token,
            profile_has_capability(&session.access_profile, AdminCapability::AccessManage),
        )
        .into_string(),
    )
    .into_response()
}

#[derive(Deserialize)]
struct KnownHostDelete {
    csrf_token: String,
    key_type: String,
    confirm_reset: Option<String>,
}

async fn delete_known_host(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path((hostname, port)): Path<(String, i64)>,
    peer: OptionalPeer,
    Form(form): Form<KnownHostDelete>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AccessManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    if !trust_reset_confirmed(form.confirm_reset.as_deref()) {
        record_audit(
            &state,
            Some(&session),
            source_ip(peer.as_ref()),
            "known_host.reset",
            "known_host",
            None,
            Some(&format!("{hostname}:{port}")),
            "failure",
            Some(serde_json::json!({
                "reason": "confirmation_required",
                "key_type": form.key_type
            })),
        )
        .await;
        return (StatusCode::BAD_REQUEST, "trust reset confirmation required").into_response();
    }
    match state
        .db
        .delete_known_host(&hostname, port, &form.key_type)
        .await
    {
        Ok(()) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "known_host.reset",
                "known_host",
                None,
                Some(&format!("{hostname}:{port}")),
                "success",
                Some(serde_json::json!({"key_type": form.key_type})),
            )
            .await;
            Redirect::to("/known-hosts").into_response()
        }
        Err(err) => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "known_host.reset",
                "known_host",
                None,
                Some(&format!("{hostname}:{port}")),
                "failure",
                Some(serde_json::json!({
                    "reason": "database",
                    "key_type": form.key_type
                })),
            )
            .await;
            admin_db_error("delete known host", err)
        }
    }
}

async fn sessions(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::SessionsRead) {
        return resp;
    }
    let sessions = match state.db.list_sessions(100).await {
        Ok(sessions) => sessions,
        Err(err) => return admin_db_error("list sessions", err),
    };
    let admin_events = match state.db.list_audit_events(100).await {
        Ok(events) => events,
        Err(err) => return admin_db_error("list admin audit events", err),
    };
    let active_session_ids = state.active_sessions.active_ids().await;
    Html(
        html::sessions(
            t,
            &sessions,
            &admin_events,
            &active_session_ids,
            &session.csrf_token,
            profile_has_capability(&session.access_profile, AdminCapability::AccessManage),
        )
        .into_string(),
    )
    .into_response()
}

async fn terminate_session(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    peer: OptionalPeer,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AccessManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }

    match state.active_sessions.terminate(&id).await {
        TerminateSessionResult::Signaled => {
            if let Err(err) = state
                .db
                .finish_session(&id, "terminated", Some(TERMINATED_BY_ADMIN))
                .await
            {
                record_audit(
                    &state,
                    Some(&session),
                    source_ip(peer.as_ref()),
                    "session.terminate",
                    "session",
                    Some(&id),
                    None,
                    "failure",
                    Some(serde_json::json!({"reason": "database"})),
                )
                .await;
                return admin_db_error("terminate session", err);
            }
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "session.terminate",
                "session",
                Some(&id),
                None,
                "success",
                None,
            )
            .await;
            Redirect::to("/sessions").into_response()
        }
        TerminateSessionResult::NotFound => {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "session.terminate",
                "session",
                Some(&id),
                None,
                "failure",
                Some(serde_json::json!({"reason": "not_found"})),
            )
            .await;
            (StatusCode::NOT_FOUND, "active session not found").into_response()
        }
    }
}

async fn terminate_all_sessions(
    State(state): State<AdminState>,
    headers: HeaderMap,
    peer: OptionalPeer,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AccessManage) {
        return resp;
    }
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }

    let session_ids = state.active_sessions.terminate_all().await;
    for session_id in &session_ids {
        if let Err(err) = state
            .db
            .finish_session(session_id, "terminated", Some(TERMINATED_BY_ADMIN))
            .await
        {
            record_audit(
                &state,
                Some(&session),
                source_ip(peer.as_ref()),
                "session.terminate_all",
                "session",
                None,
                None,
                "failure",
                Some(serde_json::json!({
                    "reason": "database",
                    "signaled": session_ids.len()
                })),
            )
            .await;
            return admin_db_error("terminate sessions", err);
        }
    }
    record_audit(
        &state,
        Some(&session),
        source_ip(peer.as_ref()),
        "session.terminate_all",
        "session",
        None,
        None,
        "success",
        Some(serde_json::json!({"signaled": session_ids.len()})),
    )
    .await;
    Redirect::to("/sessions").into_response()
}

#[derive(Deserialize)]
struct SetLanguageQuery {
    lang: String,
    redirect: Option<String>,
}

async fn set_language(Query(query): Query<SetLanguageQuery>) -> Response {
    let locale = locale_from_code(&query.lang).unwrap_or(super::i18n::Locale::En);
    let redirect = safe_redirect(query.redirect.as_deref()).unwrap_or("/");
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, language_cookie(locale.cookie_value())),
            (header::LOCATION, redirect.to_string()),
        ],
    )
        .into_response()
}

#[derive(Deserialize)]
struct ExportQuery {
    format: Option<String>,
}

async fn export_assets(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Query(query): Query<ExportQuery>,
) -> Response {
    let Ok(_session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let format = query
        .format
        .as_deref()
        .map(TransferFormat::parse)
        .transpose()
        .ok()
        .flatten()
        .unwrap_or(TransferFormat::Json);
    let assets = match state.db.list_assets().await {
        Ok(assets) => assets,
        Err(err) => return admin_db_error("list assets for export", err),
    };
    let body = match transfer::export_assets(&assets, format) {
        Ok(body) => body,
        Err(err) => return admin_db_error("export assets", err),
    };
    download_response("hop-assets", format, body)
}

async fn export_credentials(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Query(query): Query<ExportQuery>,
) -> Response {
    let Ok(_session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let format = query
        .format
        .as_deref()
        .map(TransferFormat::parse)
        .transpose()
        .ok()
        .flatten()
        .unwrap_or(TransferFormat::Json);
    let credentials = match state.db.list_credentials().await {
        Ok(credentials) => credentials,
        Err(err) => return admin_db_error("list credentials for export", err),
    };
    let body = match transfer::export_credentials(&credentials, format) {
        Ok(body) => body,
        Err(err) => return admin_db_error("export credentials", err),
    };
    download_response("hop-credentials", format, body)
}

async fn import_page(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AssetsManage) {
        return resp;
    }
    Html(html::import_export(t, &session.csrf_token, None).into_string()).into_response()
}

async fn import_data(
    State(state): State<AdminState>,
    headers: HeaderMap,
    peer: OptionalPeer,
    mut multipart: Multipart,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = capability_guard(&headers, &session, AdminCapability::AssetsManage) {
        return resp;
    }

    let mut csrf_token = String::new();
    let mut kind = TransferKind::Assets;
    let mut format = TransferFormat::Csv;
    let mut policy = ConflictPolicy::Skip;
    let mut payload = Vec::new();
    let mut summary = ImportSummary::default();

    while let Ok(Some(field)) = multipart.next_field().await {
        let Some(name) = field.name().map(ToString::to_string) else {
            continue;
        };
        match name.as_str() {
            "csrf_token" => {
                csrf_token = field.text().await.unwrap_or_default();
            }
            "kind" => {
                if let Ok(parsed) = TransferKind::parse(&field.text().await.unwrap_or_default()) {
                    kind = parsed;
                }
            }
            "format" => {
                if let Ok(parsed) = TransferFormat::parse(&field.text().await.unwrap_or_default()) {
                    format = parsed;
                }
            }
            "on_conflict" => {
                if let Ok(parsed) = ConflictPolicy::parse(&field.text().await.unwrap_or_default()) {
                    policy = parsed;
                }
            }
            "file" => {
                payload = field.bytes().await.unwrap_or_default().to_vec();
            }
            _ => {}
        }
    }

    if let Some(resp) = csrf_guard(&state, &session, &csrf_token).await {
        return resp;
    }

    let input = match String::from_utf8(payload) {
        Ok(input) => input,
        Err(err) => {
            summary.record_error(err.to_string());
            record_import_audit(
                &state,
                &session,
                peer.as_ref(),
                kind,
                format,
                policy,
                &summary,
            )
            .await;
            return Html(html::import_export(t, &session.csrf_token, Some(&summary)).into_string())
                .into_response();
        }
    };

    let result = match kind {
        TransferKind::Assets => transfer::import_assets(&state.db, &input, format, policy).await,
        TransferKind::Credentials => {
            transfer::import_credentials(&state.db, &input, format, policy).await
        }
    };
    match result {
        Ok(summary) => {
            record_import_audit(
                &state,
                &session,
                peer.as_ref(),
                kind,
                format,
                policy,
                &summary,
            )
            .await;
            Html(html::import_export(t, &session.csrf_token, Some(&summary)).into_string())
                .into_response()
        }
        Err(err) => {
            summary.record_error(err.to_string());
            record_import_audit(
                &state,
                &session,
                peer.as_ref(),
                kind,
                format,
                policy,
                &summary,
            )
            .await;
            Html(html::import_export(t, &session.csrf_token, Some(&summary)).into_string())
                .into_response()
        }
    }
}

async fn record_import_audit(
    state: &AdminState,
    session: &AuthenticatedSession,
    peer: Option<&Extension<ConnectInfo<SocketAddr>>>,
    kind: TransferKind,
    format: TransferFormat,
    policy: ConflictPolicy,
    summary: &ImportSummary,
) {
    record_audit(
        state,
        Some(session),
        source_ip(peer),
        "data.import",
        match kind {
            TransferKind::Assets => "asset_batch",
            TransferKind::Credentials => "credential_batch",
        },
        None,
        None,
        if summary.errors.is_empty() {
            "success"
        } else {
            "failure"
        },
        Some(serde_json::json!({
            "kind": format!("{kind:?}").to_ascii_lowercase(),
            "format": format!("{format:?}").to_ascii_lowercase(),
            "conflict_policy": format!("{policy:?}").to_ascii_lowercase(),
            "imported": summary.imported,
            "skipped": summary.skipped,
            "overwritten": summary.overwritten,
            "error_count": summary.errors.len()
        })),
    )
    .await;
}

fn request_l10n(headers: &HeaderMap) -> &'static super::i18n::L10n {
    l10n(resolve_locale(headers))
}

fn admin_db_error(context: &'static str, err: impl std::fmt::Display) -> Response {
    warn!(%context, error = %err, "admin database operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "admin database operation failed",
    )
        .into_response()
}

fn language_cookie(value: &str) -> String {
    format!("{LOCALE_COOKIE}={value}; Max-Age=31536000; Path=/; SameSite=Lax; HttpOnly")
}

fn safe_redirect(value: Option<&str>) -> Option<&str> {
    let value = value?;
    (value.starts_with('/') && !value.starts_with("//")).then_some(value)
}

fn settings_password_error_message(
    t: &L10n,
    err: bootstrap::AdminPasswordChangeError,
) -> &'static str {
    match err {
        bootstrap::AdminPasswordChangeError::CurrentPasswordInvalid => {
            t.settings_current_password_invalid
        }
        bootstrap::AdminPasswordChangeError::NewPasswordEmpty => t.settings_new_password_empty,
        bootstrap::AdminPasswordChangeError::NewPasswordTooShort => {
            t.settings_new_password_too_short
        }
        bootstrap::AdminPasswordChangeError::ConfirmationMismatch => {
            t.settings_password_confirmation_mismatch
        }
    }
}

fn settings_password_error_code(err: bootstrap::AdminPasswordChangeError) -> &'static str {
    match err {
        bootstrap::AdminPasswordChangeError::CurrentPasswordInvalid => "current_password_invalid",
        bootstrap::AdminPasswordChangeError::NewPasswordEmpty => "new_password_empty",
        bootstrap::AdminPasswordChangeError::NewPasswordTooShort => "new_password_too_short",
        bootstrap::AdminPasswordChangeError::ConfirmationMismatch => "confirmation_mismatch",
    }
}

fn download_response(name: &str, format: TransferFormat, body: String) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, format.content_type().to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}.{}\"", format.extension()),
            ),
        ],
        body,
    )
        .into_response()
}

fn collect_tags(assets: &[hop_core::Asset]) -> Vec<String> {
    let mut tags = assets
        .iter()
        .flat_map(|asset| asset.tags.iter().cloned())
        .collect::<Vec<_>>();
    tags.sort();
    tags.dedup();
    tags
}

fn filter_assets(
    assets: &[hop_core::Asset],
    selected_tag: Option<&str>,
    query: Option<&str>,
) -> Vec<hop_core::Asset> {
    let selected_tag = selected_tag.map(str::trim).filter(|tag| !tag.is_empty());
    let query = query.map(str::trim).filter(|query| !query.is_empty());

    assets
        .iter()
        .filter(|asset| {
            selected_tag.is_none_or(|tag| asset.tags.iter().any(|asset_tag| asset_tag == tag))
                && query.is_none_or(|query| asset_matches_query(asset, query))
        })
        .cloned()
        .collect()
}

fn credential_is_in_use(assets: &[hop_core::Asset], credential_id: &str) -> bool {
    assets
        .iter()
        .any(|asset| asset.credential_id.as_deref() == Some(credential_id))
}

fn trust_reset_confirmed(value: Option<&str>) -> bool {
    value == Some("yes")
}

fn assets_return_to(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| {
            (*value == "/assets" || value.starts_with("/assets?")) && !value.contains(['\r', '\n'])
        })
        .unwrap_or("/assets")
        .to_string()
}

fn asset_matches_query(asset: &hop_core::Asset, query: &str) -> bool {
    let query = query.to_ascii_lowercase();
    [
        asset.name.as_str(),
        asset.hostname.as_str(),
        asset.description.as_deref().unwrap_or_default(),
        asset.protocol.as_str(),
        asset.preset.as_deref().unwrap_or_default(),
    ]
    .into_iter()
    .chain(asset.tags.iter().map(String::as_str))
    .any(|value| value.to_ascii_lowercase().contains(&query))
        || asset.port.to_string().contains(&query)
}

fn parse_bulk_tags_body(body: &[u8]) -> Result<BulkTagsForm> {
    let mut form = BulkTagsForm {
        csrf_token: String::new(),
        asset_ids: Vec::new(),
        tags: None,
    };
    for (key, value) in form_urlencoded::parse(body) {
        match key.as_ref() {
            "csrf_token" => form.csrf_token = value.into_owned(),
            "asset_ids" => form.asset_ids.push(value.into_owned()),
            "tags" => form.tags = Some(value.into_owned()),
            _ => {}
        }
    }
    ensure!(!form.csrf_token.is_empty(), "missing CSRF token");
    Ok(form)
}

#[cfg(test)]
mod tests {
    use super::bootstrap::AdminPasswordChangeError;
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use std::fs;
    use tower::ServiceExt;

    async fn test_admin_state() -> AdminState {
        AdminState {
            db: HopDb::in_memory().await.unwrap(),
            master_key: Arc::new(MasterKey::from_bytes([0; 32])),
            active_sessions: ActiveSessionRegistry::default(),
            sessions: AdminSessions::default(),
            ssh_port: 2222,
            ssh_bind: "127.0.0.1:2222".parse().unwrap(),
            admin_bind: "127.0.0.1:8080".parse().unwrap(),
            started_at: "2026-07-28T00:00:00Z".to_string(),
            started_instant: Instant::now(),
            cookie_secure: false,
        }
    }

    fn test_assets() -> Vec<hop_core::Asset> {
        vec![
            hop_core::Asset {
                id: "web-prod".to_string(),
                name: "Web-PROD-01".to_string(),
                protocol: "ssh".to_string(),
                preset: None,
                hostname: "app01.internal".to_string(),
                port: 22,
                description: Some("Primary frontend".to_string()),
                tags: vec!["prod".to_string(), "frontend".to_string()],
                credential_id: None,
                created_at: None,
                updated_at: None,
            },
            hop_core::Asset {
                id: "windows-ops".to_string(),
                name: "Windows operations".to_string(),
                protocol: "tcp".to_string(),
                preset: Some("rdp".to_string()),
                hostname: "10.0.0.8".to_string(),
                port: 3389,
                description: Some("Remote desktop".to_string()),
                tags: vec!["prod".to_string(), "windows".to_string()],
                credential_id: None,
                created_at: None,
                updated_at: None,
            },
            hop_core::Asset {
                id: "database-stage".to_string(),
                name: "Reporting database".to_string(),
                protocol: "tcp".to_string(),
                preset: Some("postgresql".to_string()),
                hostname: "db.stage.internal".to_string(),
                port: 5432,
                description: None,
                tags: vec!["staging".to_string(), "database".to_string()],
                credential_id: None,
                created_at: None,
                updated_at: None,
            },
        ]
    }

    fn filtered_asset_ids(tag: Option<&str>, query: Option<&str>) -> Vec<String> {
        filter_assets(&test_assets(), tag, query)
            .into_iter()
            .map(|asset| asset.id)
            .collect()
    }

    #[test]
    fn asset_filter_supports_query_only() {
        assert_eq!(
            filtered_asset_ids(None, Some("remote desktop")),
            vec!["windows-ops"]
        );
    }

    #[test]
    fn asset_filter_searches_every_supported_field() {
        for (query, expected_id) in [
            ("Web-PROD-01", "web-prod"),
            ("app01.internal", "web-prod"),
            ("22", "web-prod"),
            ("Primary frontend", "web-prod"),
            ("frontend", "web-prod"),
            ("ssh", "web-prod"),
            ("postgresql", "database-stage"),
        ] {
            assert_eq!(
                filtered_asset_ids(None, Some(query)),
                vec![expected_id],
                "query {query:?} did not match the expected field"
            );
        }
    }

    #[test]
    fn asset_filter_supports_tag_only() {
        assert_eq!(
            filtered_asset_ids(Some("prod"), None),
            vec!["web-prod", "windows-ops"]
        );
    }

    #[test]
    fn asset_filter_combines_query_and_tag() {
        assert_eq!(
            filtered_asset_ids(Some("prod"), Some("rdp")),
            vec!["windows-ops"]
        );
        assert!(filtered_asset_ids(Some("staging"), Some("rdp")).is_empty());
    }

    #[test]
    fn asset_filter_treats_blank_query_as_no_query() {
        assert_eq!(
            filtered_asset_ids(None, Some(" \t ")),
            vec!["web-prod", "windows-ops", "database-stage"]
        );
    }

    #[test]
    fn asset_filter_trims_query_and_ignores_ascii_case() {
        assert_eq!(
            filtered_asset_ids(None, Some("  wEb-PrOd  ")),
            vec!["web-prod"]
        );
    }

    #[test]
    fn asset_return_location_accepts_only_asset_list_urls() {
        assert_eq!(
            assets_return_to(Some("/assets?tag=prod&q=api")),
            "/assets?tag=prod&q=api"
        );
        assert_eq!(assets_return_to(Some("/credentials")), "/assets");
        assert_eq!(assets_return_to(Some("//example.com")), "/assets");
        assert_eq!(assets_return_to(Some("/assets\r\nLocation: /")), "/assets");
    }

    #[test]
    fn credential_usage_guard_detects_assigned_assets() {
        let mut assets = test_assets();
        assets[1].credential_id = Some("cred-1".to_string());

        assert!(credential_is_in_use(&assets, "cred-1"));
        assert!(!credential_is_in_use(&assets, "cred-2"));
    }

    #[test]
    fn known_host_reset_requires_explicit_confirmation() {
        assert!(trust_reset_confirmed(Some("yes")));
        assert!(!trust_reset_confirmed(None));
        assert!(!trust_reset_confirmed(Some("true")));
    }

    #[test]
    fn dashboard_probe_uses_loopback_for_wildcard_listeners() {
        assert_eq!(
            probe_address("0.0.0.0:2222".parse().unwrap()),
            "127.0.0.1:2222".parse().unwrap()
        );
        assert_eq!(
            probe_address("[::]:2222".parse().unwrap()),
            "[::1]:2222".parse().unwrap()
        );
        assert_eq!(
            probe_address("10.0.0.8:2222".parse().unwrap()),
            "10.0.0.8:2222".parse().unwrap()
        );
    }

    #[test]
    fn dashboard_source_failure_isolated_to_named_source() {
        let mut errors = Vec::new();
        let value: Vec<String> = dashboard_value::<Vec<String>, _>(
            Err(std::io::Error::other("database busy")),
            "recent sessions",
            &mut errors,
        );

        assert!(value.is_empty());
        assert_eq!(errors, vec!["recent sessions"]);
    }

    #[tokio::test]
    async fn admin_audit_records_identity_source_and_safe_details() {
        let state = test_admin_state().await;
        let session = AuthenticatedSession {
            token: "session-token".to_string(),
            csrf_token: "csrf-token".to_string(),
            admin_id: "local-admin".to_string(),
            admin_label: "Local admin".to_string(),
            access_profile: hop_core::ADMIN_PROFILE_OWNER.to_string(),
            must_change_password: false,
        };

        record_audit(
            &state,
            Some(&session),
            Some("127.0.0.1".to_string()),
            "credential.update",
            "credential",
            Some("credential-1"),
            Some("production"),
            "success",
            Some(serde_json::json!({"auth_type": "key"})),
        )
        .await;

        let events = state.db.list_audit_events(10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].actor_id.as_deref(), Some("local-admin"));
        assert_eq!(events[0].actor_label, "Local admin");
        assert_eq!(events[0].source_ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(
            events[0].details_json.as_deref(),
            Some(r#"{"auth_type":"key"}"#)
        );
        assert!(!events[0]
            .details_json
            .as_deref()
            .unwrap()
            .contains("secret"));
    }

    #[tokio::test]
    async fn admin_router_serves_built_frontend_assets() {
        let dist = tempfile::tempdir().unwrap();
        fs::create_dir_all(dist.path().join("assets")).unwrap();
        fs::write(
            dist.path().join("assets/admin.css"),
            b"body{background:#0d1117}",
        )
        .unwrap();

        let app = admin_router_with_static_dir(test_admin_state().await, dist.path());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin-static/assets/admin.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/css"
        );
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"body{background:#0d1117}");
    }

    #[tokio::test]
    async fn login_stays_password_only_until_a_second_admin_is_added() {
        let state = test_admin_state().await;
        let dist = tempfile::tempdir().unwrap();
        let first = admin_router_with_static_dir(state.clone(), dist.path())
            .oneshot(
                Request::builder()
                    .uri("/login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let first_body = to_bytes(first.into_body(), 64 * 1024).await.unwrap();
        let first_html = String::from_utf8(first_body.to_vec()).unwrap();
        assert!(!first_html.contains(r#"name="username""#));

        state
            .db
            .add_admin_user(NewAdminUser {
                username: "ops".to_string(),
                display_name: "Operations".to_string(),
                password_hash: "test-hash".to_string(),
                access_profile: hop_core::ADMIN_PROFILE_OPERATOR.to_string(),
                must_change_password: true,
            })
            .await
            .unwrap();
        let team = admin_router_with_static_dir(state, dist.path())
            .oneshot(
                Request::builder()
                    .uri("/login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let team_body = to_bytes(team.into_body(), 64 * 1024).await.unwrap();
        let team_html = String::from_utf8(team_body.to_vec()).unwrap();
        assert!(team_html.contains(r#"name="username""#));
        assert!(team_html.contains(super::super::i18n::EN.login_team_intro));
    }

    #[tokio::test]
    async fn denied_mutations_explain_the_required_task_access() {
        let session = AuthenticatedSession {
            token: "viewer-token".to_string(),
            csrf_token: "viewer-csrf".to_string(),
            admin_id: "viewer".to_string(),
            admin_label: "Viewer".to_string(),
            access_profile: hop_core::ADMIN_PROFILE_VIEWER.to_string(),
            must_change_password: false,
        };

        let response = capability_guard(
            &HeaderMap::new(),
            &session,
            AdminCapability::CredentialsManage,
        )
        .expect("viewer should not manage credentials");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let rendered = String::from_utf8(body.to_vec()).unwrap();
        assert!(rendered.contains(super::super::i18n::EN.permission_denied_heading));
        assert!(rendered.contains(super::super::i18n::EN.admin_profile_viewer));
        assert!(!rendered.contains("RBAC"));
    }

    #[test]
    fn admin_static_dist_dir_uses_runtime_env_override() {
        let dist = tempfile::tempdir().unwrap();
        std::env::set_var("HOP_ADMIN_STATIC_DIR", dist.path());

        let resolved = admin_static_dist_dir();

        std::env::remove_var("HOP_ADMIN_STATIC_DIR");
        assert_eq!(resolved, dist.path());
    }

    #[test]
    fn bulk_tags_form_parses_repeated_asset_ids() {
        let form = parse_bulk_tags_body(
            b"csrf_token=csrf-123&asset_ids=asset-1&asset_ids=asset-2&tags=prod%2Cweb",
        )
        .unwrap();

        assert_eq!(form.csrf_token, "csrf-123");
        assert_eq!(form.asset_ids, vec!["asset-1", "asset-2"]);
        assert_eq!(form.tags.as_deref(), Some("prod,web"));
    }

    #[test]
    fn key_access_form_parses_repeated_asset_ids_and_empty_selection() {
        let form = parse_key_access_form(
            b"csrf_token=csrf-123&name=laptop&public_key=ssh-ed25519+AAAA&asset_access_mode=restricted&asset_id=asset-1&asset_id=asset-2",
        )
        .unwrap();
        assert_eq!(form.csrf_token, "csrf-123");
        assert_eq!(form.asset_access_mode, "restricted");
        assert_eq!(form.asset_ids, vec!["asset-1", "asset-2"]);

        let empty = parse_key_access_form(
            b"csrf_token=csrf-123&name=laptop&public_key=ssh-ed25519+AAAA&asset_access_mode=restricted",
        )
        .unwrap();
        assert!(empty.asset_ids.is_empty());
    }

    #[test]
    fn asset_form_clears_credentials_for_rdp_protocol() {
        let asset = new_asset_from_form(AssetForm {
            csrf_token: "csrf-123".to_string(),
            return_to: None,
            name: "win-rdp".to_string(),
            protocol: Some("rdp".to_string()),
            preset: None,
            hostname: "10.0.2.20".to_string(),
            port: 3389,
            description: None,
            tags: Some("windows,rdp".to_string()),
            credential_id: Some("cred-1".to_string()),
        })
        .unwrap();

        assert_eq!(asset.protocol, "tcp");
        assert_eq!(asset.preset.as_deref(), Some("rdp"));
        assert_eq!(asset.tags, vec!["windows", "rdp"]);
        assert!(asset.credential_id.is_none());
    }

    #[tokio::test]
    async fn terminate_session_route_signals_registry_and_records_audit() {
        let state = test_admin_state().await;
        let ssh_session = state
            .db
            .start_session(hop_core::NewSession {
                key_finger: "SHA256:alice".to_string(),
                key_name: Some("alice".to_string()),
                mode: "exec".to_string(),
                asset_name: Some("prod-api-01".to_string()),
                target_host: Some("10.42.1.12".to_string()),
                target_port: Some(22),
                client_ip: Some("10.42.0.18".to_string()),
            })
            .await
            .unwrap();
        let (terminate_tx, mut terminate_rx) = tokio::sync::mpsc::unbounded_channel();
        state
            .active_sessions
            .register(ssh_session.id.clone(), terminate_tx)
            .await;
        let token = state
            .sessions
            .create(
                "local-admin",
                "Local admin",
                hop_core::ADMIN_PROFILE_OWNER,
                false,
            )
            .await;
        let admin_session = state.sessions.authenticate(&token).await.unwrap();
        let cookie = session_cookie(&token, false);
        let app = admin_router_with_static_dir(state.clone(), tempfile::tempdir().unwrap().path());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/sessions/{}/terminate", ssh_session.id))
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!(
                        "csrf_token={}",
                        admin_session.csrf_token
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(terminate_rx.recv().await, Some(()));
        let finished = state
            .db
            .get_session(&ssh_session.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(finished.status, "terminated");
        assert_eq!(finished.error.as_deref(), Some(TERMINATED_BY_ADMIN));
        assert!(finished.ended_at.is_some());
        let events = state.db.list_audit_events(10).await.unwrap();
        assert!(events.iter().any(|event| {
            event.action == "session.terminate"
                && event.target_id.as_deref() == Some(ssh_session.id.as_str())
                && event.result == "success"
        }));
    }

    #[test]
    fn settings_password_errors_map_to_localized_messages() {
        assert_eq!(
            settings_password_error_message(
                &super::super::i18n::EN,
                AdminPasswordChangeError::CurrentPasswordInvalid
            ),
            super::super::i18n::EN.settings_current_password_invalid
        );
        assert_eq!(
            settings_password_error_message(
                &super::super::i18n::EN,
                AdminPasswordChangeError::NewPasswordEmpty
            ),
            super::super::i18n::EN.settings_new_password_empty
        );
        assert_eq!(
            settings_password_error_message(
                &super::super::i18n::EN,
                AdminPasswordChangeError::NewPasswordTooShort
            ),
            super::super::i18n::EN.settings_new_password_too_short
        );
        assert_eq!(
            settings_password_error_message(
                &super::super::i18n::EN,
                AdminPasswordChangeError::ConfirmationMismatch
            ),
            super::super::i18n::EN.settings_password_confirmation_mismatch
        );
    }
}
