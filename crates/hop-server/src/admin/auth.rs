use std::{collections::HashSet, sync::Arc};

use axum::http::{header, HeaderMap};
use cookie::{Cookie, SameSite};
use rand::{distributions::Alphanumeric, Rng};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Default)]
pub struct AdminSessions {
    inner: Arc<Mutex<HashSet<String>>>,
}

impl AdminSessions {
    pub async fn create(&self) -> String {
        let token: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(48)
            .map(char::from)
            .collect();
        self.inner.lock().await.insert(token.clone());
        token
    }

    pub async fn remove(&self, token: &str) {
        self.inner.lock().await.remove(token);
    }

    pub async fn contains(&self, token: &str) -> bool {
        self.inner.lock().await.contains(token)
    }
}

pub const ADMIN_COOKIE: &str = "hop_admin";

pub fn session_cookie(token: &str) -> String {
    Cookie::build((ADMIN_COOKIE, token.to_string()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .build()
        .to_string()
}

pub fn clear_cookie() -> String {
    Cookie::build((ADMIN_COOKIE, ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .max_age(cookie::time::Duration::seconds(0))
        .build()
        .to_string()
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

pub async fn require_login(headers: &HeaderMap, sessions: &AdminSessions) -> bool {
    match cookie_token(headers) {
        Some(token) => sessions.contains(&token).await,
        None => false,
    }
}
