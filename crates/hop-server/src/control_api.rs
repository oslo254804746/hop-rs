use std::{net::SocketAddr, sync::Arc};

use anyhow::{bail, Context, Result};
use axum::{
    extract::{Path as AxumPath, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use hop_core::{
    encrypt_envelope, new_id, ApplyOptions, ApplySummary, Asset, AssetAccessMode, AuthType,
    Catalog, CatalogError, CatalogErrorCode, Credential, HopCoreError, Manifest, MasterKey,
    NewAsset, NewAuthorizedKey, NewCredential, ResourceOwnership, Session,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::ssh::session_registry::{ActiveSessionRegistry, TerminateSessionResult};

#[derive(Clone)]
pub struct ControlApiState {
    catalog: Catalog,
    master_key: Arc<MasterKey>,
    token_hash: [u8; 32],
    active_sessions: ActiveSessionRegistry,
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
        })
    }
}

pub async fn serve(
    bind: SocketAddr,
    state: ControlApiState,
    cors_allowlist: &[String],
) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("bind Control API at {bind}"))?;
    axum::serve(listener, router(state).layer(cors_layer(cors_allowlist)?)).await?;
    Ok(())
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
        .route(
            "/api/v1/assets/{id}",
            put(update_asset).delete(delete_asset),
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
        .route("/api/v1/catalog/revision", get(revision))
        .route("/api/v1/config/validate", post(validate))
        .route("/api/v1/config/diff", post(diff))
        .route("/api/v1/config/apply", post(apply))
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
    Ok(Json(asset_view(&state.catalog, asset).await?))
}

async fn delete_asset(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    authenticate(&state, &headers)?;
    state.catalog.delete_asset(&id).await?;
    Ok(StatusCode::NO_CONTENT)
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
    Ok(Json(credential_view(&state.catalog, credential).await?))
}

async fn delete_credential(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    authenticate(&state, &headers)?;
    state.catalog.delete_credential(&id).await?;
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
    let view = access_key_view(&state.catalog, key).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

async fn delete_access_key(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    authenticate(&state, &headers)?;
    state.catalog.delete_authorized_key(&id).await?;
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
            true
        }
        TerminateSessionResult::NotFound => false,
    };
    Ok(Json(TerminateResponse { id, terminated }))
}

async fn validate(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    Json(request): Json<ValidateRequest>,
) -> ApiResult<Json<ValidationResponse>> {
    authenticate(&state, &headers)?;
    let manifest = request.manifest.parse()?;
    if request.offline {
        manifest.validate_offline()?;
    } else {
        state.catalog.validate_manifest(&manifest).await?;
    }
    Ok(Json(ValidationResponse {
        valid: true,
        offline: request.offline,
    }))
}

async fn diff(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    Json(request): Json<DiffRequest>,
) -> ApiResult<Json<ApplySummary>> {
    authenticate(&state, &headers)?;
    let manifest = request.manifest.parse()?;
    Ok(Json(
        state
            .catalog
            .diff(
                &manifest,
                &request.source_id,
                &state.master_key,
                request.prune,
            )
            .await?,
    ))
}

async fn apply(
    State(state): State<ControlApiState>,
    headers: HeaderMap,
    Json(request): Json<ApplyRequest>,
) -> ApiResult<Json<ApplySummary>> {
    authenticate(&state, &headers)?;
    let ApplyRequest {
        manifest,
        source_id,
        base_revision,
        prune,
        dry_run,
    } = request;
    let manifest = match manifest.parse() {
        Ok(manifest) => manifest,
        Err(error) => {
            if !dry_run {
                let _ = state
                    .catalog
                    .record_apply_failure(&source_id, "api", &error)
                    .await;
            }
            return Err(error.into());
        }
    };
    let result = state
        .catalog
        .apply(
            &manifest,
            &source_id,
            &state.master_key,
            ApplyOptions {
                base_revision: Some(base_revision),
                prune,
                dry_run,
            },
        )
        .await;
    match result {
        Ok(summary) => Ok(Json(summary)),
        Err(error) => {
            if !dry_run {
                let _ = state
                    .catalog
                    .record_apply_failure(&source_id, "api", &error)
                    .await;
            }
            Err(error.into())
        }
    }
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
#[serde(rename_all = "lowercase")]
enum ManifestFormat {
    Yaml,
    Toml,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestPayload {
    content: String,
    format: ManifestFormat,
}

impl ManifestPayload {
    fn parse(self) -> std::result::Result<Manifest, CatalogError> {
        match self.format {
            ManifestFormat::Yaml => Manifest::from_yaml(&self.content),
            ManifestFormat::Toml => Manifest::from_toml(&self.content),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidateRequest {
    #[serde(flatten)]
    manifest: ManifestPayload,
    #[serde(default)]
    offline: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiffRequest {
    #[serde(flatten)]
    manifest: ManifestPayload,
    source_id: String,
    #[serde(default)]
    prune: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyRequest {
    #[serde(flatten)]
    manifest: ManifestPayload,
    source_id: String,
    base_revision: i64,
    #[serde(default)]
    prune: bool,
    #[serde(default)]
    dry_run: bool,
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
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    credential_id: Option<String>,
}

impl From<AssetWriteRequest> for NewAsset {
    fn from(request: AssetWriteRequest) -> Self {
        Self {
            name: request.name,
            protocol: request.protocol,
            hostname: request.hostname,
            port: request.port,
            description: request.description,
            tags: request.tags,
            credential_id: request.credential_id,
        }
    }
}

fn default_asset_protocol() -> String {
    "ssh".to_string()
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

#[derive(Serialize)]
struct StatusResponse {
    status: &'static str,
    version: &'static str,
    catalog_revision: i64,
}

#[derive(Serialize)]
struct RevisionResponse {
    revision: i64,
}

#[derive(Serialize)]
struct ValidationResponse {
    valid: bool,
    offline: bool,
}

#[derive(Serialize)]
struct AssetView {
    #[serde(flatten)]
    asset: Asset,
    ownership: ResourceOwnership,
}

async fn asset_view(catalog: &Catalog, asset: Asset) -> ApiResult<AssetView> {
    let ownership = catalog
        .resource_ownership("asset", &asset.id)
        .await
        .map_err(ApiError::internal)?;
    Ok(AssetView { asset, ownership })
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
    async fn api_apply_uses_catalog_revision_and_never_returns_secret_material() {
        let directory = tempfile::tempdir().unwrap();
        let password = directory.path().join("password");
        fs::write(&password, "api-secret-value").unwrap();
        let manifest = format!(
            "api_version: hop/v1alpha1\ncredentials:\n  root:\n    type: password\n    username: root\n    password:\n      file: {}\n",
            password.display()
        );
        let state = ControlApiState::new(
            Catalog::in_memory().await.unwrap(),
            Arc::new(MasterKey::generate()),
            "test-token",
            ActiveSessionRegistry::default(),
        )
        .unwrap();
        let app = router(state);
        let payload = serde_json::json!({
            "content": manifest,
            "format": "yaml",
            "source_id": "api-test",
            "base_revision": 0
        });

        let applied = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/config/apply")
                    .header(header::AUTHORIZATION, "Bearer test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(applied.status(), StatusCode::OK);

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
                key_finger: key.fingerprint.clone(),
                key_name: Some(key.name.clone()),
                mode: "direct".to_string(),
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
