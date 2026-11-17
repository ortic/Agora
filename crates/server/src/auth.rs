//! Authentication: password hashing, sessions, API tokens and request extractors.

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, header};
use base64::Engine;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::app::AppState;
use crate::error::ApiError;

pub const SESSION_COOKIE: &str = "agora_session";
/// Mutating cookie-authenticated requests must carry this header (CSRF protection: custom
/// headers can't be sent cross-site without a CORS preflight, which we never allow).
pub const CSRF_HEADER: &str = "x-agora";
pub const API_TOKEN_PREFIX: &str = "agt_";

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: Uuid,
    pub email: Option<String>,
    pub name: String,
    pub role: String,
    pub status: String,
}

impl User {
    pub fn is_admin(&self) -> bool {
        matches!(self.role.as_str(), "owner" | "admin")
    }

    /// May read the audit log, usage of everyone and others' conversations.
    pub fn can_audit(&self) -> bool {
        matches!(self.role.as_str(), "owner" | "admin" | "auditor")
    }

    /// For audit rows: who it was at the time.
    pub fn label(&self) -> String {
        self.email.clone().unwrap_or_else(|| self.name.clone())
    }
}

pub const USER_COLUMNS: &str = "id, email, name, role, status";

/// Argon2 uses ~19 MiB and noticeable CPU per hash: bound concurrent hashes (DoS protection).
static HASH_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

/// Failed-login throttle (shared by all replicas): per key, 10 failures per 15 minutes.
pub async fn login_allowed(state: &AppState, key: &str) -> bool {
    let failures: Option<i32> = sqlx::query_scalar("SELECT failures FROM login_attempts WHERE email = $1 AND window_start > now() - interval '15 minutes'")
        .bind(key)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    failures.unwrap_or(0) < 10
}

pub async fn record_login_failure(state: &AppState, key: &str) {
    let _ = sqlx::query(
        "INSERT INTO login_attempts (email, failures, window_start) VALUES ($1, 1, now())
         ON CONFLICT (email) DO UPDATE SET failures = CASE WHEN login_attempts.window_start < now() - interval '15 minutes' THEN 1 ELSE login_attempts.failures + 1 END,
                                           window_start = CASE WHEN login_attempts.window_start < now() - interval '15 minutes' THEN now() ELSE login_attempts.window_start END",
    )
    .bind(key)
    .execute(&state.db)
    .await;
}

pub async fn clear_login_failures(state: &AppState, key: &str) {
    let _ = sqlx::query("DELETE FROM login_attempts WHERE email = $1").bind(key).execute(&state.db).await;
}

pub async fn hash_password(password: String) -> anyhow::Result<String> {
    let _slot = HASH_SLOTS.acquire().await?;
    tokio::task::spawn_blocking(move || Argon2::default().hash_password(password.as_bytes()).map(|h| h.to_string()).map_err(|e| anyhow::anyhow!("hash: {e}")))
        .await?
}

pub async fn verify_password(password: String, hash: String) -> bool {
    let Ok(_slot) = HASH_SLOTS.acquire().await else { return false };
    tokio::task::spawn_blocking(move || PasswordHash::new(&hash).map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok()).unwrap_or(false))
        .await
        .unwrap_or(false)
}

pub fn random_token() -> String {
    let mut buf = [0u8; 32];
    rand::fill(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Client address and user agent of a request (for sessions and the audit log).
#[derive(Debug, Clone, Default)]
pub struct Client {
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

impl Client {
    pub fn from_headers(state: &AppState, headers: &HeaderMap, peer: Option<std::net::SocketAddr>) -> Self {
        let forwarded = state
            .config
            .trust_proxy
            .then(|| headers.get("x-forwarded-for").or_else(|| headers.get("x-real-ip")))
            .flatten()
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|v| v.trim().chars().take(64).collect::<String>());
        let ip = forwarded.or_else(|| peer.map(|p| p.ip().to_string()));
        let user_agent = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(|s| s.chars().take(300).collect());
        Client { ip, user_agent }
    }
}

impl FromRequestParts<AppState> for Client {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let peer = parts.extensions.get::<axum::extract::ConnectInfo<std::net::SocketAddr>>().map(|c| c.0);
        Ok(Client::from_headers(state, &parts.headers, peer))
    }
}

/// Creates a session and returns the `Set-Cookie` header value.
pub async fn create_session(state: &AppState, user_id: Uuid, client: &Client) -> Result<String, ApiError> {
    let token = random_token();
    let days = state.config.session_days;
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at, user_agent, ip) VALUES ($1, $2, now() + make_interval(days => $3), $4, $5)")
        .bind(token_hash(&token))
        .bind(user_id)
        .bind(days as i32)
        .bind(&client.user_agent)
        .bind(&client.ip)
        .execute(&state.db)
        .await?;
    sqlx::query("UPDATE users SET last_login_at = now() WHERE id = $1").bind(user_id).execute(&state.db).await?;
    Ok(session_cookie(&token, days * 86400, state.config.secure_cookies))
}

pub fn session_cookie(token: &str, max_age: i64, secure: bool) -> String {
    format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{}", if secure { "; Secure" } else { "" })
}

pub(crate) fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then_some(v)
    })
}

pub fn session_token(headers: &HeaderMap) -> Option<&str> {
    cookie_value(headers, SESSION_COOKIE)
}

async fn user_from_session(state: &AppState, token: &str) -> Result<Option<User>, ApiError> {
    let user: Option<User> = sqlx::query_as(
        "SELECT u.id, u.email, u.name, u.role, u.status
           FROM sessions s JOIN users u ON u.id = s.user_id
          WHERE s.token_hash = $1 AND s.expires_at > now() AND u.status = 'active'
            AND ($2 = 0 OR s.last_seen_at > now() - make_interval(mins => $2))",
    )
    .bind(token_hash(token))
    .bind(state.config.session_idle_minutes.min(i32::MAX as u64) as i32)
    .fetch_optional(&state.db)
    .await?;
    if let Some(u) = &user {
        // sliding activity timestamps (cheap, best effort, 5-minute steps)
        let _ = sqlx::query("UPDATE sessions SET last_seen_at = now() WHERE token_hash = $1 AND last_seen_at < now() - interval '5 minutes'")
            .bind(token_hash(token))
            .execute(&state.db)
            .await;
        let _ = sqlx::query("UPDATE users SET last_active_at = now() WHERE id = $1 AND (last_active_at IS NULL OR last_active_at < now() - interval '5 minutes')")
            .bind(u.id)
            .execute(&state.db)
            .await;
    }
    Ok(user)
}

/// Marks requests authenticated with an API token (not a browser session).
#[derive(Debug, Clone, Copy)]
pub struct ViaToken;

pub async fn user_from_api_token(state: &AppState, token: &str) -> Result<Option<User>, ApiError> {
    let user: Option<User> = sqlx::query_as(
        "SELECT u.id, u.email, u.name, u.role, u.status FROM api_tokens t JOIN users u ON u.id = t.user_id
         WHERE t.token_hash = $1 AND (t.expires_at IS NULL OR t.expires_at > now()) AND u.status = 'active'",
    )
    .bind(token_hash(token))
    .fetch_optional(&state.db)
    .await?;
    if user.is_some() {
        let _ = sqlx::query("UPDATE api_tokens SET last_used_at = now() WHERE token_hash = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')")
            .bind(token_hash(token))
            .execute(&state.db)
            .await;
    }
    Ok(user)
}

/// The authenticated user, if any. Never fails for anonymous requests.
#[derive(Debug, Clone)]
pub struct MaybeUser(pub Option<User>);

/// The authenticated user; rejects anonymous requests with 401.
#[derive(Debug, Clone)]
pub struct CurrentUser(pub User);

/// An administrator (owner or admin).
#[derive(Debug, Clone)]
pub struct AdminUser(pub User);

/// Someone who may read audit data (owner, admin or auditor).
#[derive(Debug, Clone)]
pub struct AuditorUser(pub User);

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if let Some(cached) = parts.extensions.get::<MaybeUser>() {
            return Ok(cached.clone());
        }
        // API token (Authorization: Bearer agt_...): no CSRF concerns
        if let Some(auth) = parts.headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok())
            && let Some(tok) = auth.strip_prefix("Bearer ").map(str::trim)
        {
            let Some(user) = user_from_api_token(state, tok).await? else {
                return Err(ApiError::Unauthorized);
            };
            parts.extensions.insert(ViaToken);
            let mu = MaybeUser(Some(user));
            parts.extensions.insert(mu.clone());
            return Ok(mu);
        }
        let user = match session_token(&parts.headers) {
            Some(tok) => user_from_session(state, tok).await?,
            None => None,
        };
        if user.is_some() && is_mutating(&parts.method) && parts.headers.get(CSRF_HEADER).is_none() {
            return Err(ApiError::BadRequest(format!("missing {CSRF_HEADER} header")));
        }
        let mu = MaybeUser(user);
        parts.extensions.insert(mu.clone());
        Ok(mu)
    }
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        match MaybeUser::from_request_parts(parts, state).await?.0 {
            Some(u) => Ok(CurrentUser(u)),
            None => Err(ApiError::Unauthorized),
        }
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let CurrentUser(u) = CurrentUser::from_request_parts(parts, state).await?;
        if u.is_admin() { Ok(AdminUser(u)) } else { Err(ApiError::Forbidden) }
    }
}

impl FromRequestParts<AppState> for AuditorUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let CurrentUser(u) = CurrentUser::from_request_parts(parts, state).await?;
        if u.can_audit() { Ok(AuditorUser(u)) } else { Err(ApiError::Forbidden) }
    }
}

fn is_mutating(m: &Method) -> bool {
    !matches!(*m, Method::GET | Method::HEAD | Method::OPTIONS)
}
