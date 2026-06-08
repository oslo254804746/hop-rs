use std::{net::SocketAddr, sync::Arc};

use anyhow::{ensure, Result};
use axum::{
    body::Bytes,
    extract::{Form, Multipart, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use hop_core::{
    encrypt_envelope, new_id, protocol_supports_managed_credentials, validate_asset_protocol,
    validate_credential_material, validate_tcp_port, AuthType, HopDb, MasterKey, NewAsset,
    NewAuthorizedKey, NewCredential, ASSET_PROTOCOL_SSH,
};
use serde::Deserialize;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;

use super::{
    auth::{
        clear_cookie, cookie_token, require_login, session_cookie, AdminSessions,
        AuthenticatedSession,
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
    pub sessions: AdminSessions,
    pub ssh_port: u16,
}

pub async fn serve_admin(
    bind: SocketAddr,
    ssh_bind: SocketAddr,
    db: HopDb,
    master_key: Arc<MasterKey>,
) -> Result<()> {
    let state = AdminState {
        db,
        master_key,
        sessions: AdminSessions::default(),
        ssh_port: ssh_bind.port(),
    };
    let app = Router::new()
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
        .route("/import", get(import_page).post(import_data))
        .route("/settings", get(settings).post(update_settings))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = TcpListener::bind(bind).await?;
    info!(%bind, "admin web listening");
    axum::serve(listener, app).await?;
    Ok(())
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

async fn index(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let t = request_l10n(&headers);
    let Ok(_session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let assets = state.db.list_assets().await.unwrap_or_default();
    let credentials = state.db.list_credentials().await.unwrap_or_default();
    let keys = state.db.list_authorized_keys().await.unwrap_or_default();
    let sessions = state.db.list_sessions(10).await.unwrap_or_default();
    Html(
        html::overview(
            t,
            assets.len(),
            credentials.len(),
            keys.len(),
            sessions.len(),
        )
        .into_string(),
    )
    .into_response()
}

async fn login_page(headers: HeaderMap) -> Html<String> {
    let t = request_l10n(&headers);
    Html(html::login(t, None).into_string())
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
}

async fn login(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let t = request_l10n(&headers);
    match bootstrap::verify_admin_password(&state.db, &form.password).await {
        Ok(true) => {
            let token = state.sessions.create().await;
            (
                StatusCode::SEE_OTHER,
                [
                    (header::SET_COOKIE, session_cookie(&token)),
                    (header::LOCATION, "/".to_string()),
                ],
            )
                .into_response()
        }
        _ => Html(html::login(t, Some(t.login_invalid_password)).into_string()).into_response(),
    }
}

async fn logout(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie_token(&headers) {
        state.sessions.remove(&token).await;
    }
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, clear_cookie()),
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
    Html(html::settings(t, &session.csrf_token, None).into_string()).into_response()
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
    Form(form): Form<SettingsForm>,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    match bootstrap::change_admin_password(
        &state.db,
        &form.current_password,
        &form.new_password,
        &form.confirm_password,
    )
    .await
    {
        Ok(Ok(())) => {
            state.sessions.remove(&session.token).await;
            (
                StatusCode::SEE_OTHER,
                [
                    (header::SET_COOKIE, clear_cookie()),
                    (header::LOCATION, "/login".to_string()),
                ],
            )
                .into_response()
        }
        Ok(Err(err)) => Html(
            html::settings(
                t,
                &session.csrf_token,
                Some(settings_password_error_message(t, err)),
            )
            .into_string(),
        )
        .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to change admin password",
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct AssetsQuery {
    tag: Option<String>,
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
    let all_assets = state.db.list_assets().await.unwrap_or_default();
    let all_tags = collect_tags(&all_assets);
    let assets = filter_assets_by_tag(&all_assets, query.tag.as_deref());
    let credentials = state.db.list_credentials().await.unwrap_or_default();
    Html(
        html::assets(
            t,
            &assets,
            &credentials,
            &session.csrf_token,
            query.tag.as_deref(),
            &all_tags,
            state.ssh_port,
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
    name: String,
    protocol: Option<String>,
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
    Form(form): Form<AssetForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let Some(asset) = new_asset_from_form(form) else {
        return Redirect::to("/assets").into_response();
    };
    let _ = state.db.add_asset(asset).await;
    Redirect::to("/assets").into_response()
}

async fn edit_asset(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let Ok(Some(asset)) = state.db.get_asset_by_id(&id).await else {
        return Redirect::to("/assets").into_response();
    };
    let credentials = state.db.list_credentials().await.unwrap_or_default();
    let all_assets = state.db.list_assets().await.unwrap_or_default();
    let all_tags = collect_tags(&all_assets);
    Html(
        html::edit_asset(
            t,
            &asset,
            &credentials,
            &session.csrf_token,
            &all_tags,
            state.ssh_port,
        )
        .into_string(),
    )
    .into_response()
}

async fn update_asset(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<AssetForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let Some(asset) = new_asset_from_form(form) else {
        return Redirect::to("/assets").into_response();
    };
    let _ = state.db.update_asset(&id, asset).await;
    Redirect::to("/assets").into_response()
}

async fn delete_asset(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let _ = state.db.delete_asset(&id).await;
    Redirect::to("/assets").into_response()
}

async fn bulk_update_asset_tags(
    State(state): State<AdminState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(form) = parse_bulk_tags_body(&body) else {
        return (StatusCode::BAD_REQUEST, "invalid bulk tag form").into_response();
    };
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let tags = parse_tags(form.tags);
    for asset_id in form.asset_ids {
        if let Ok(Some(asset)) = state.db.get_asset_by_id(&asset_id).await {
            let _ = state
                .db
                .update_asset(
                    &asset.id,
                    NewAsset {
                        name: asset.name,
                        protocol: asset.protocol,
                        hostname: asset.hostname,
                        port: asset.port,
                        description: asset.description,
                        tags: tags.clone(),
                        credential_id: asset.credential_id,
                    },
                )
                .await;
        }
    }
    Redirect::to("/assets").into_response()
}

async fn credentials(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let credentials = state.db.list_credentials().await.unwrap_or_default();
    Html(html::credentials(t, &credentials, &session.csrf_token).into_string()).into_response()
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
    Form(form): Form<CredentialForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let Ok(auth_type) = AuthType::try_from(form.auth_type.as_str()) else {
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
        return Redirect::to("/credentials").into_response();
    }
    let _ = state
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
        .await;
    Redirect::to("/credentials").into_response()
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
    let Ok(Some(credential)) = state.db.get_credential(&id).await else {
        return Redirect::to("/credentials").into_response();
    };
    Html(html::edit_credential(t, &credential, &session.csrf_token).into_string()).into_response()
}

async fn update_credential(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<CredentialForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let Ok(Some(existing)) = state.db.get_credential(&id).await else {
        return Redirect::to("/credentials").into_response();
    };
    let Ok(auth_type) = AuthType::try_from(form.auth_type.as_str()) else {
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
        return Redirect::to("/credentials").into_response();
    }
    let _ = state
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
        .await;
    Redirect::to("/credentials").into_response()
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
    let protocol =
        validate_asset_protocol(form.protocol.as_deref().unwrap_or(ASSET_PROTOCOL_SSH)).ok()?;
    validate_tcp_port(form.port).ok()?;
    let credential_id = if protocol_supports_managed_credentials(&protocol) {
        form.credential_id.filter(|value| !value.trim().is_empty())
    } else {
        None
    };
    Some(NewAsset {
        name: form.name,
        protocol,
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
    Form(form): Form<CsrfForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let _ = state.db.delete_credential(&id).await;
    Redirect::to("/credentials").into_response()
}

async fn keys(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let keys = state.db.list_authorized_keys().await.unwrap_or_default();
    Html(html::keys(t, &keys, &session.csrf_token).into_string()).into_response()
}

#[derive(Deserialize)]
struct KeyForm {
    csrf_token: String,
    name: String,
    public_key: String,
}

async fn create_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Form(form): Form<KeyForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    if let Ok((public_key, fingerprint)) = parse_public_key_line(&form.public_key) {
        let _ = state
            .db
            .add_authorized_key(NewAuthorizedKey::new(form.name, public_key, fingerprint))
            .await;
    }
    Redirect::to("/keys").into_response()
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
    let Ok(Some(key)) = state.db.get_authorized_key_by_id(&id).await else {
        return Redirect::to("/keys").into_response();
    };
    Html(html::edit_key(t, &key, &session.csrf_token).into_string()).into_response()
}

async fn update_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<KeyForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    if let Ok((public_key, fingerprint)) = parse_public_key_line(&form.public_key) {
        let _ = state
            .db
            .update_authorized_key(
                &id,
                NewAuthorizedKey::new(form.name, public_key, fingerprint),
            )
            .await;
    }
    Redirect::to("/keys").into_response()
}

async fn deactivate_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let _ = state.db.set_authorized_key_active(&id, false).await;
    Redirect::to("/keys").into_response()
}

async fn activate_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let _ = state.db.set_authorized_key_active(&id, true).await;
    Redirect::to("/keys").into_response()
}

async fn delete_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let _ = state.db.delete_authorized_key(&id).await;
    Redirect::to("/keys").into_response()
}

async fn known_hosts(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let hosts = state.db.list_known_hosts().await.unwrap_or_default();
    Html(html::known_hosts(t, &hosts, &session.csrf_token).into_string()).into_response()
}

#[derive(Deserialize)]
struct KnownHostDelete {
    csrf_token: String,
    key_type: String,
}

async fn delete_known_host(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path((hostname, port)): Path<(String, i64)>,
    Form(form): Form<KnownHostDelete>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(resp) = csrf_guard(&state, &session, &form.csrf_token).await {
        return resp;
    }
    let _ = state
        .db
        .delete_known_host(&hostname, port, &form.key_type)
        .await;
    Redirect::to("/known-hosts").into_response()
}

async fn sessions(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let t = request_l10n(&headers);
    let Ok(_session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let sessions = state.db.list_sessions(100).await.unwrap_or_default();
    Html(html::sessions(t, &sessions).into_string()).into_response()
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
    let assets = state.db.list_assets().await.unwrap_or_default();
    download_response(
        "hop-assets",
        format,
        transfer::export_assets(&assets, format).unwrap_or_default(),
    )
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
    let credentials = state.db.list_credentials().await.unwrap_or_default();
    download_response(
        "hop-credentials",
        format,
        transfer::export_credentials(&credentials, format).unwrap_or_default(),
    )
}

async fn import_page(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    Html(html::import_export(t, &session.csrf_token, None).into_string()).into_response()
}

async fn import_data(
    State(state): State<AdminState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let t = request_l10n(&headers);
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };

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
            Html(html::import_export(t, &session.csrf_token, Some(&summary)).into_string())
                .into_response()
        }
        Err(err) => {
            summary.record_error(err.to_string());
            Html(html::import_export(t, &session.csrf_token, Some(&summary)).into_string())
                .into_response()
        }
    }
}

fn request_l10n(headers: &HeaderMap) -> &'static super::i18n::L10n {
    l10n(resolve_locale(headers))
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
        bootstrap::AdminPasswordChangeError::ConfirmationMismatch => {
            t.settings_password_confirmation_mismatch
        }
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

fn filter_assets_by_tag(assets: &[hop_core::Asset], tag: Option<&str>) -> Vec<hop_core::Asset> {
    let Some(tag) = tag.map(str::trim).filter(|tag| !tag.is_empty()) else {
        return assets.to_vec();
    };
    assets
        .iter()
        .filter(|asset| asset.tags.iter().any(|asset_tag| asset_tag == tag))
        .cloned()
        .collect()
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
    fn asset_form_clears_credentials_for_rdp_protocol() {
        let asset = new_asset_from_form(AssetForm {
            csrf_token: "csrf-123".to_string(),
            name: "win-rdp".to_string(),
            protocol: Some("rdp".to_string()),
            hostname: "10.0.2.20".to_string(),
            port: 3389,
            description: None,
            tags: Some("windows,rdp".to_string()),
            credential_id: Some("cred-1".to_string()),
        })
        .unwrap();

        assert_eq!(asset.protocol, "rdp");
        assert_eq!(asset.tags, vec!["windows", "rdp"]);
        assert!(asset.credential_id.is_none());
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
                AdminPasswordChangeError::ConfirmationMismatch
            ),
            super::super::i18n::EN.settings_password_confirmation_mismatch
        );
    }
}
