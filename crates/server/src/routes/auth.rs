use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::app::{AppState, new_id};
use crate::audit;
use crate::auth::{
    API_TOKEN_PREFIX, Client, CurrentUser, MaybeUser, USER_COLUMNS, User, create_session, hash_password, random_token, session_cookie, session_token,
    token_hash, verify_password,
};
use crate::error::{ApiError, ApiResult};

pub async fn setup_status(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(&state.db).await?;
    Ok(Json(json!({ "needsSetup": n == 0 })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupRequest {
    organization: Option<String>,
    name: String,
    email: String,
    password: String,
}

pub fn validate_password(p: &str) -> ApiResult<()> {
    if p.chars().count() < 10 {
        return Err(ApiError::bad("the password needs at least 10 characters"));
    }
    Ok(())
}

pub fn validate_email(e: &str) -> ApiResult<String> {
    let e = e.trim().to_lowercase();
    if e.len() > 254 || !e.contains('@') || e.starts_with('@') || e.ends_with('@') || e.contains(char::is_whitespace) {
        return Err(ApiError::bad("invalid e-mail address"));
    }
    Ok(e)
}

/// First-run setup: creates the owner account. Only works while there are no users.
pub async fn setup(State(state): State<AppState>, client: Client, Json(req): Json<SetupRequest>) -> ApiResult<Response> {
    validate_password(&req.password)?;
    let email = validate_email(&req.email)?;
    if req.name.trim().is_empty() {
        return Err(ApiError::bad("your name is required"));
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(&state.db).await?;
    if n > 0 {
        return Err(ApiError::Conflict("already set up".into()));
    }
    let hash = hash_password(req.password).await?;
    let mut tx = state.db.begin().await?;
    // serialize concurrent setup attempts
    sqlx::query("LOCK TABLE users IN EXCLUSIVE MODE").execute(&mut *tx).await?;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(&mut *tx).await?;
    if n > 0 {
        return Err(ApiError::Conflict("already set up".into()));
    }
    let uid = new_id();
    sqlx::query("INSERT INTO users (id, email, name, password_hash, role) VALUES ($1, $2, $3, $4, 'owner')")
        .bind(uid)
        .bind(&email)
        .bind(req.name.trim())
        .bind(hash)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if let Some(org) = req.organization.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
        let mut s = crate::settings::load(&state).await?;
        s.name = org.chars().take(80).collect();
        crate::settings::save(&state, &s).await?;
    }
    let user = load_user(&state, uid).await?;
    audit::log(&state, Some(&user), Some(&client), "setup.completed", Some(("user", uid.to_string())), json!({ "email": email })).await;
    let cookie = create_session(&state, uid, &client).await?;
    Ok(([(header::SET_COOKIE, cookie)], Json(json!({ "ok": true }))).into_response())
}

#[derive(Deserialize)]
pub struct LoginRequest {
    email: String,
    password: String,
}

pub async fn login(State(state): State<AppState>, client: Client, Json(req): Json<LoginRequest>) -> ApiResult<Response> {
    let has_directory = state.providers.iter().any(|p| matches!(p, crate::sso::Provider::Ldap(_)));
    if state.config.disable_password_login && !has_directory {
        return Err(ApiError::bad("password sign-in is turned off; use single sign-on"));
    }
    let email = req.email.trim().to_lowercase();
    if !crate::auth::login_allowed(&state, &email).await {
        return Err(ApiError::bad("too many failed attempts, try again in 15 minutes"));
    }
    let row: Option<(Uuid, Option<String>, String)> =
        sqlx::query_as("SELECT id, password_hash, status FROM users WHERE lower(email) = $1").bind(&email).fetch_optional(&state.db).await?;
    // constant-ish time: always verify (against a real hash for unknown users)
    static DUMMY: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
    let dummy = DUMMY.get_or_try_init(|| hash_password("agora-timing-dummy".into())).await?.clone();
    let mut method = "password";
    let mut user_id = match row {
        Some((id, Some(hash), status)) if status == "active" && !state.config.disable_password_login => {
            verify_password(req.password.clone(), hash).await.then_some(id)
        }
        _ => {
            let _ = verify_password(req.password.clone(), dummy).await;
            None
        }
    };
    if user_id.is_none() {
        user_id = ldap_login(&state, &req.email, &req.password).await?;
        method = "ldap";
    }
    match user_id {
        Some(id) => {
            crate::auth::clear_login_failures(&state, &email).await;
            let cookie = create_session(&state, id, &client).await?;
            let user = load_user(&state, id).await?;
            audit::log(&state, Some(&user), Some(&client), "auth.login", None, json!({ "method": method })).await;
            Ok(([(header::SET_COOKIE, cookie)], Json(json!({ "ok": true }))).into_response())
        }
        None => {
            crate::auth::record_login_failure(&state, &email).await;
            audit::log(&state, None, Some(&client), "auth.login_failed", None, json!({ "email": email })).await;
            Err(ApiError::bad("wrong e-mail or password"))
        }
    }
}

pub async fn logout(State(state): State<AppState>, MaybeUser(user): MaybeUser, client: Client, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(tok) = session_token(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1").bind(token_hash(tok)).execute(&state.db).await?;
    }
    if let Some(u) = &user {
        audit::log(&state, Some(u), Some(&client), "auth.logout", None, json!({})).await;
    }
    Ok(([(header::SET_COOKIE, session_cookie("", 0, state.config.secure_cookies))], StatusCode::NO_CONTENT).into_response())
}

/// `GET /api/me`: who is signed in, and what the app needs to know about the installation.
pub async fn me(State(state): State<AppState>, MaybeUser(user): MaybeUser) -> ApiResult<Json<Value>> {
    let s = crate::settings::load(&state).await?;
    let web = s.web_search.available();
    Ok(Json(json!({
        "user": user,
        "organization": s.name,
        "notice": s.notice,
        "webSearch": web,
        "defaultModelId": s.default_model_id,
        "allowUserSystemPrompt": s.allow_user_system_prompt,
        "desktop": match &user {
            Some(u) => crate::desktop::may_use_desktop(&state, u.id).await,
            None => false,
        },
    })))
}

/// `GET /api/me/usage`: the signed-in person's own usage and budget.
pub async fn my_usage(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<Value>> {
    let s = crate::settings::load(&state).await?;
    let budget = crate::usage::budget(&state, user.id, &s).await?;
    let by_model: Vec<(String, i64, i64, i64, f64)> = sqlx::query_as(
        "SELECT model_name, count(*), sum(input_tokens + cache_read_tokens + cache_write_tokens)::int8, sum(output_tokens)::int8, sum(cost_usd)::float8
           FROM usage_events WHERE user_id = $1 AND created_at >= date_trunc('month', now()) GROUP BY model_name ORDER BY 5 DESC",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    let days: Vec<(chrono::NaiveDate, f64, i64)> = sqlx::query_as(
        "SELECT created_at::date, sum(cost_usd)::float8, count(*) FROM usage_events WHERE user_id = $1 AND created_at > now() - interval '30 days' GROUP BY 1 ORDER BY 1",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "budget": budget,
        "models": by_model.iter().map(|(m, n, i, o, c)| json!({ "model": m, "requests": n, "inputTokens": i, "outputTokens": o, "costUsd": c })).collect::<Vec<_>>(),
        "days": days.iter().map(|(d, c, n)| json!({ "day": d, "costUsd": c, "requests": n })).collect::<Vec<_>>(),
    })))
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TokenRow {
    id: Uuid,
    name: String,
    created_at: chrono::DateTime<chrono::Utc>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn list_tokens(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<Vec<TokenRow>>> {
    Ok(Json(
        sqlx::query_as::<_, TokenRow>("SELECT id, name, created_at, expires_at, last_used_at FROM api_tokens WHERE user_id = $1 ORDER BY created_at DESC")
            .bind(user.id)
            .fetch_all(&state.db)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateToken {
    name: String,
    expires_in_days: Option<i32>,
}

/// Creates a personal API token (for the OpenAI-compatible API). The token is shown once.
pub async fn create_token(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    client: Client,
    Json(req): Json<CreateToken>,
) -> ApiResult<Json<Value>> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad("give the token a name"));
    }
    let token = format!("{API_TOKEN_PREFIX}{}", random_token());
    let id = new_id();
    sqlx::query(
        "INSERT INTO api_tokens (id, user_id, name, token_hash, expires_at)
         VALUES ($1, $2, $3, $4, CASE WHEN $5::int IS NULL THEN NULL ELSE now() + make_interval(days => $5::int) END)",
    )
    .bind(id)
    .bind(user.id)
    .bind(name)
    .bind(token_hash(&token))
    .bind(req.expires_in_days.filter(|d| *d > 0))
    .execute(&state.db)
    .await?;
    audit::log(
        &state,
        Some(&user),
        Some(&client),
        "token.created",
        Some(("token", id.to_string())),
        json!({ "name": name, "expiresInDays": req.expires_in_days }),
    )
    .await;
    Ok(Json(json!({ "id": id, "token": token })))
}

pub async fn delete_token(State(state): State<AppState>, CurrentUser(user): CurrentUser, client: Client, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let n = sqlx::query("DELETE FROM api_tokens WHERE id = $1 AND user_id = $2").bind(id).bind(user.id).execute(&state.db).await?.rows_affected();
    if n > 0 {
        audit::log(&state, Some(&user), Some(&client), "token.deleted", Some(("token", id.to_string())), json!({})).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn load_user(state: &AppState, id: Uuid) -> ApiResult<User> {
    sqlx::query_as::<_, User>(sqlx::AssertSqlSafe(format!("SELECT {USER_COLUMNS} FROM users WHERE id = $1")))
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("user"))
}

/// Tries the configured LDAP providers.
async fn ldap_login(state: &AppState, username: &str, password: &str) -> ApiResult<Option<Uuid>> {
    for p in state.providers.iter() {
        let crate::sso::Provider::Ldap(c) = p else { continue };
        match crate::sso::ldap::authenticate(c, username, password).await {
            Ok(Some(ext)) => {
                return match crate::sso::sign_in(state, p, ext).await {
                    Ok(id) => Ok(Some(id)),
                    Err(crate::sso::SignInError::Denied(m)) => Err(ApiError::BadRequest(m)),
                    Err(crate::sso::SignInError::Other(e)) => Err(ApiError::Internal(e)),
                };
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(provider = %c.common.id, "LDAP login failed: {e:#}"),
        }
    }
    Ok(None)
}
