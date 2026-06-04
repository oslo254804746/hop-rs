use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum::{
    extract::{Form, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use hop_core::{
    encrypt_envelope, new_id, validate_credential_material, validate_tcp_port, AuthType, HopDb,
    MasterKey, NewAsset, NewAuthorizedKey, NewCredential,
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
    local_cli::parse_public_key_line,
};

#[derive(Clone)]
pub struct AdminState {
    pub db: HopDb,
    pub master_key: Arc<MasterKey>,
    pub sessions: AdminSessions,
}

pub async fn serve_admin(bind: SocketAddr, db: HopDb, master_key: Arc<MasterKey>) -> Result<()> {
    let state = AdminState {
        db,
        master_key,
        sessions: AdminSessions::default(),
    };
    let app = Router::new()
        .route("/", get(index))
        .route("/login", get(login_page).post(login))
        .route("/logout", get(logout))
        .route("/assets", get(assets).post(create_asset))
        .route("/assets/{id}/edit", get(edit_asset))
        .route("/assets/{id}", post(update_asset))
        .route("/assets/{id}/delete", post(delete_asset))
        .route("/credentials", get(credentials).post(create_credential))
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
    let Ok(_session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let assets = state.db.list_assets().await.unwrap_or_default();
    let credentials = state.db.list_credentials().await.unwrap_or_default();
    let keys = state.db.list_authorized_keys().await.unwrap_or_default();
    let sessions = state.db.list_sessions(10).await.unwrap_or_default();
    Html(html::overview(assets.len(), credentials.len(), keys.len(), sessions.len()).into_string())
        .into_response()
}

async fn login_page() -> Html<String> {
    Html(html::login(None).into_string())
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
}

async fn login(State(state): State<AdminState>, Form(form): Form<LoginForm>) -> Response {
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
        _ => Html(html::login(Some("Invalid password")).into_string()).into_response(),
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

async fn assets(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let assets = state.db.list_assets().await.unwrap_or_default();
    let credentials = state.db.list_credentials().await.unwrap_or_default();
    Html(html::assets(&assets, &credentials, &session.csrf_token).into_string()).into_response()
}

#[derive(Deserialize)]
struct CsrfForm {
    csrf_token: String,
}

#[derive(Deserialize)]
struct AssetForm {
    csrf_token: String,
    name: String,
    hostname: String,
    port: i64,
    description: Option<String>,
    tags: Option<String>,
    credential_id: Option<String>,
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
    let tags = parse_tags(form.tags);
    let credential_id = form.credential_id.filter(|value| !value.trim().is_empty());
    if validate_tcp_port(form.port).is_err() {
        return Redirect::to("/assets").into_response();
    }
    let _ = state
        .db
        .add_asset(NewAsset {
            name: form.name,
            hostname: form.hostname,
            port: form.port,
            description: form.description.filter(|v| !v.trim().is_empty()),
            tags,
            credential_id,
        })
        .await;
    Redirect::to("/assets").into_response()
}

async fn edit_asset(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let Ok(Some(asset)) = state.db.get_asset_by_id(&id).await else {
        return Redirect::to("/assets").into_response();
    };
    let credentials = state.db.list_credentials().await.unwrap_or_default();
    Html(html::edit_asset(&asset, &credentials, &session.csrf_token).into_string()).into_response()
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
    let tags = parse_tags(form.tags);
    let credential_id = form.credential_id.filter(|value| !value.trim().is_empty());
    if validate_tcp_port(form.port).is_err() {
        return Redirect::to("/assets").into_response();
    }
    let _ = state
        .db
        .update_asset(
            &id,
            NewAsset {
                name: form.name,
                hostname: form.hostname,
                port: form.port,
                description: form.description.filter(|v| !v.trim().is_empty()),
                tags,
                credential_id,
            },
        )
        .await;
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

async fn credentials(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let credentials = state.db.list_credentials().await.unwrap_or_default();
    Html(html::credentials(&credentials, &session.csrf_token).into_string()).into_response()
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
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let Ok(Some(credential)) = state.db.get_credential(&id).await else {
        return Redirect::to("/credentials").into_response();
    };
    Html(html::edit_credential(&credential, &session.csrf_token).into_string()).into_response()
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
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let keys = state.db.list_authorized_keys().await.unwrap_or_default();
    Html(html::keys(&keys, &session.csrf_token).into_string()).into_response()
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
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let Ok(Some(key)) = state.db.get_authorized_key_by_id(&id).await else {
        return Redirect::to("/keys").into_response();
    };
    Html(html::edit_key(&key, &session.csrf_token).into_string()).into_response()
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
    let Ok(session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let hosts = state.db.list_known_hosts().await.unwrap_or_default();
    Html(html::known_hosts(&hosts, &session.csrf_token).into_string()).into_response()
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
    let Ok(_session) = guard(&headers, &state).await else {
        return Redirect::to("/login").into_response();
    };
    let sessions = state.db.list_sessions(100).await.unwrap_or_default();
    Html(html::sessions(&sessions).into_string()).into_response()
}
