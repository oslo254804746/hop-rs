use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::http::{header, HeaderMap};
use cookie::{Cookie, SameSite};
use rand::{distributions::Alphanumeric, Rng};
use tokio::sync::Mutex;

use hop_core::{ADMIN_PROFILE_OPERATOR, ADMIN_PROFILE_OWNER, ADMIN_PROFILE_VIEWER};

const SESSION_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminCapability {
    InventoryRead,
    AssetsManage,
    CredentialsManage,
    AccessManage,
    SessionsRead,
    AdminsManage,
}

pub fn profile_has_capability(profile: &str, capability: AdminCapability) -> bool {
    match profile {
        ADMIN_PROFILE_OWNER => true,
        ADMIN_PROFILE_OPERATOR => matches!(
            capability,
            AdminCapability::InventoryRead
                | AdminCapability::AssetsManage
                | AdminCapability::CredentialsManage
                | AdminCapability::AccessManage
                | AdminCapability::SessionsRead
        ),
        ADMIN_PROFILE_VIEWER => matches!(
            capability,
            AdminCapability::InventoryRead | AdminCapability::SessionsRead
        ),
        _ => false,
    }
}

#[derive(Debug)]
struct AdminSession {
    csrf_token: String,
    admin_id: String,
    admin_label: String,
    access_profile: String,
    must_change_password: bool,
    last_seen: Instant,
}

#[derive(Debug, Clone)]
pub struct AdminSessions {
    inner: Arc<Mutex<HashMap<String, AdminSession>>>,
    ttl: Duration,
}

impl Default for AdminSessions {
    fn default() -> Self {
        Self::with_ttl(SESSION_TTL)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedSession {
    pub token: String,
    pub csrf_token: String,
    pub admin_id: String,
    pub admin_label: String,
    pub access_profile: String,
    pub must_change_password: bool,
}

impl AdminSessions {
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl,
        }
    }

    pub async fn create(
        &self,
        admin_id: &str,
        admin_label: &str,
        access_profile: &str,
        must_change_password: bool,
    ) -> String {
        let token = random_token();
        let csrf_token = random_token();
        self.inner.lock().await.insert(
            token.clone(),
            AdminSession {
                csrf_token,
                admin_id: admin_id.to_string(),
                admin_label: admin_label.to_string(),
                access_profile: access_profile.to_string(),
                must_change_password,
                last_seen: Instant::now(),
            },
        );
        token
    }

    pub async fn remove(&self, token: &str) {
        self.inner.lock().await.remove(token);
    }

    pub async fn remove_admin(&self, admin_id: &str) {
        self.inner
            .lock()
            .await
            .retain(|_, session| session.admin_id != admin_id);
    }

    pub async fn authenticate(&self, token: &str) -> Option<AuthenticatedSession> {
        let now = Instant::now();
        let mut sessions = self.inner.lock().await;
        sessions.retain(|_, session| now.duration_since(session.last_seen) <= self.ttl);
        let session = sessions.get_mut(token)?;
        session.last_seen = now;
        Some(AuthenticatedSession {
            token: token.to_string(),
            csrf_token: session.csrf_token.clone(),
            admin_id: session.admin_id.clone(),
            admin_label: session.admin_label.clone(),
            access_profile: session.access_profile.clone(),
            must_change_password: session.must_change_password,
        })
    }

    pub async fn validate_csrf(&self, token: &str, csrf_token: &str) -> bool {
        self.authenticate(token)
            .await
            .map(|session| session.csrf_token == csrf_token)
            .unwrap_or(false)
    }
}

fn random_token() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect()
}

pub const ADMIN_COOKIE: &str = "hop_admin";

pub fn session_cookie(token: &str, secure: bool) -> String {
    let mut cookie = Cookie::build((ADMIN_COOKIE, token.to_string()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict);
    if secure {
        cookie = cookie.secure(true);
    }
    cookie.build().to_string()
}

pub fn clear_cookie(secure: bool) -> String {
    let mut cookie = Cookie::build((ADMIN_COOKIE, ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .max_age(cookie::time::Duration::seconds(0));
    if secure {
        cookie = cookie.secure(true);
    }
    cookie.build().to_string()
}

pub fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let cookie = Cookie::parse(part.trim()).ok()?;
        if cookie.name() == ADMIN_COOKIE {
            return Some(cookie.value().to_string());
        }
    }
    None
}

pub async fn require_login(
    headers: &HeaderMap,
    sessions: &AdminSessions,
) -> Option<AuthenticatedSession> {
    match cookie_token(headers) {
        Some(token) => sessions.authenticate(&token).await,
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn sessions_expire_after_ttl() {
        let sessions = AdminSessions::with_ttl(Duration::from_millis(5));
        let token = sessions
            .create("local-admin", "Local admin", ADMIN_PROFILE_OWNER, false)
            .await;

        assert!(sessions.authenticate(&token).await.is_some());
        tokio::time::sleep(Duration::from_millis(15)).await;
        assert!(sessions.authenticate(&token).await.is_none());
    }

    #[tokio::test]
    async fn csrf_token_must_match_authenticated_session() {
        let sessions = AdminSessions::default();
        let token = sessions
            .create("local-admin", "Local admin", ADMIN_PROFILE_OWNER, false)
            .await;
        let session = sessions.authenticate(&token).await.unwrap();

        assert_eq!(session.admin_id, "local-admin");
        assert_eq!(session.admin_label, "Local admin");
        assert_eq!(session.access_profile, ADMIN_PROFILE_OWNER);
        assert!(!session.must_change_password);
        assert!(sessions.validate_csrf(&token, &session.csrf_token).await);
        assert!(!sessions.validate_csrf(&token, "wrong").await);
    }

    #[tokio::test]
    async fn disabling_an_admin_revokes_only_their_sessions() {
        let sessions = AdminSessions::default();
        let alice = sessions
            .create("alice", "Alice", ADMIN_PROFILE_OPERATOR, true)
            .await;
        let bob = sessions
            .create("bob", "Bob", ADMIN_PROFILE_VIEWER, false)
            .await;

        sessions.remove_admin("alice").await;

        assert!(sessions.authenticate(&alice).await.is_none());
        assert!(sessions.authenticate(&bob).await.is_some());
    }

    #[test]
    fn fixed_profiles_map_to_capabilities_without_custom_policy() {
        assert!(profile_has_capability(
            ADMIN_PROFILE_OWNER,
            AdminCapability::AdminsManage
        ));
        assert!(profile_has_capability(
            ADMIN_PROFILE_OPERATOR,
            AdminCapability::CredentialsManage
        ));
        assert!(!profile_has_capability(
            ADMIN_PROFILE_OPERATOR,
            AdminCapability::AdminsManage
        ));
        assert!(profile_has_capability(
            ADMIN_PROFILE_VIEWER,
            AdminCapability::SessionsRead
        ));
        assert!(!profile_has_capability(
            ADMIN_PROFILE_VIEWER,
            AdminCapability::AssetsManage
        ));
    }

    #[test]
    fn session_cookie_secure_flag_is_configurable() {
        assert!(!session_cookie("token", false).contains("Secure"));
        assert!(session_cookie("token", true).contains("Secure"));
        assert!(clear_cookie(true).contains("Secure"));
    }
}
