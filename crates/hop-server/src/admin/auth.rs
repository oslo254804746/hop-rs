use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::http::{header, HeaderMap};
use cookie::{Cookie, SameSite};
use rand::{distributions::Alphanumeric, Rng};
use tokio::sync::Mutex;

const SESSION_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug)]
struct AdminSession {
    csrf_token: String,
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
}

impl AdminSessions {
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl,
        }
    }

    pub async fn create(&self) -> String {
        let token = random_token();
        let csrf_token = random_token();
        self.inner.lock().await.insert(
            token.clone(),
            AdminSession {
                csrf_token,
                last_seen: Instant::now(),
            },
        );
        token
    }

    pub async fn remove(&self, token: &str) {
        self.inner.lock().await.remove(token);
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
        let token = sessions.create().await;

        assert!(sessions.authenticate(&token).await.is_some());
        tokio::time::sleep(Duration::from_millis(15)).await;
        assert!(sessions.authenticate(&token).await.is_none());
    }

    #[tokio::test]
    async fn csrf_token_must_match_authenticated_session() {
        let sessions = AdminSessions::default();
        let token = sessions.create().await;
        let session = sessions.authenticate(&token).await.unwrap();

        assert!(sessions.validate_csrf(&token, &session.csrf_token).await);
        assert!(!sessions.validate_csrf(&token, "wrong").await);
    }

    #[test]
    fn session_cookie_secure_flag_is_configurable() {
        assert!(!session_cookie("token", false).contains("Secure"));
        assert!(session_cookie("token", true).contains("Secure"));
        assert!(clear_cookie(true).contains("Secure"));
    }
}
