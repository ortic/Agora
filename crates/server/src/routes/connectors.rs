//! Knowledge sources: administrators register them, people connect their own tokens.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::app::{AppState, new_id};
use crate::audit;
use crate::auth::{AdminUser, Client, CurrentUser};
use crate::connectors::Connector;
use crate::error::{ApiError, ApiResult};

/// `GET /api/connectors`: the sources people can connect, and whether they did.
pub async fn list(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<Value>> {
    let rows: Vec<(Uuid, String, String, String, Option<String>, Option<String>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT c.id, c.name, c.kind, c.base_url, c.description, t.token_hint, t.last_used_at
           FROM connectors c LEFT JOIN connector_tokens t ON t.connector_id = c.id AND t.user_id = $1
          WHERE c.enabled ORDER BY c.name",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|(id, name, kind, base, desc, hint, used)| json!({
                "id": id, "name": name, "kind": kind, "baseUrl": base, "description": desc, "connected": hint.is_some(), "tokenHint": hint, "lastUsedAt": used,
            }))
            .collect::<Vec<_>>()
    )))
}

#[derive(Deserialize)]
pub struct TokenBody {
    token: String,
}

pub async fn connect(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    client: Client,
    Path(id): Path<Uuid>,
    Json(b): Json<TokenBody>,
) -> ApiResult<Json<Value>> {
    let c = load(&state, id).await?;
    if !c.enabled {
        return Err(ApiError::NotFound("connector"));
    }
    let token = b.token.trim();
    if token.is_empty() {
        return Err(ApiError::bad("paste a token"));
    }
    let who = crate::connectors::check_token(&c.base_url, token).await.map_err(ApiError::BadRequest)?;
    sqlx::query(
        "INSERT INTO connector_tokens (connector_id, user_id, token_enc, token_hint) VALUES ($1, $2, $3, $4)
         ON CONFLICT (connector_id, user_id) DO UPDATE SET token_enc = EXCLUDED.token_enc, token_hint = EXCLUDED.token_hint, created_at = now()",
    )
    .bind(id)
    .bind(user.id)
    .bind(state.secrets.encrypt(token)?)
    .bind(crate::crypto::hint(token))
    .execute(&state.db)
    .await?;
    audit::log(&state, Some(&user), Some(&client), "connector.connected", Some(("connector", id.to_string())), json!({ "name": c.name, "remoteUser": who }))
        .await;
    Ok(Json(json!({ "remoteUser": who })))
}

pub async fn disconnect(State(state): State<AppState>, CurrentUser(user): CurrentUser, client: Client, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let n =
        sqlx::query("DELETE FROM connector_tokens WHERE connector_id = $1 AND user_id = $2").bind(id).bind(user.id).execute(&state.db).await?.rows_affected();
    if n > 0 {
        audit::log(&state, Some(&user), Some(&client), "connector.disconnected", Some(("connector", id.to_string())), json!({})).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn load(state: &AppState, id: Uuid) -> ApiResult<Connector> {
    sqlx::query_as::<_, Connector>("SELECT id, name, kind, base_url, description, enabled FROM connectors WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("connector"))
}

// ---- administration ----------------------------------------------------------------------

pub async fn admin_list(State(state): State<AppState>, AdminUser(_): AdminUser) -> ApiResult<Json<Value>> {
    let rows: Vec<(Uuid, String, String, String, Option<String>, bool, i64)> = sqlx::query_as(
        "SELECT c.id, c.name, c.kind, c.base_url, c.description, c.enabled, (SELECT count(*) FROM connector_tokens t WHERE t.connector_id = c.id)
           FROM connectors c ORDER BY c.name",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|(id, name, kind, base, desc, enabled, n)| json!({ "id": id, "name": name, "kind": kind, "baseUrl": base, "description": desc, "enabled": enabled, "connectedUsers": n }))
            .collect::<Vec<_>>()
    )))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorBody {
    name: Option<String>,
    base_url: Option<String>,
    description: Option<String>,
    enabled: Option<bool>,
}

fn check_url(u: &str) -> ApiResult<String> {
    let u = u.trim().trim_end_matches('/').to_string();
    let p = url::Url::parse(&u).map_err(|_| ApiError::bad("the address isn't valid"))?;
    if !matches!(p.scheme(), "http" | "https") {
        return Err(ApiError::bad("the address must start with http:// or https://"));
    }
    Ok(u)
}

pub async fn admin_create(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Json(b): Json<ConnectorBody>) -> ApiResult<Json<Value>> {
    let name = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| ApiError::bad("a name is required"))?;
    let base = check_url(b.base_url.as_deref().ok_or_else(|| ApiError::bad("the wiki's address is required"))?)?;
    let id = new_id();
    sqlx::query("INSERT INTO connectors (id, name, kind, base_url, description, enabled) VALUES ($1, $2, 'wikicraft', $3, $4, $5)")
        .bind(id)
        .bind(name)
        .bind(&base)
        .bind(b.description.as_deref().map(str::trim).filter(|d| !d.is_empty()))
        .bind(b.enabled.unwrap_or(true))
        .execute(&state.db)
        .await?;
    audit::log(&state, Some(&admin), Some(&client), "connector.created", Some(("connector", id.to_string())), json!({ "name": name, "baseUrl": base })).await;
    Ok(Json(json!({ "id": id })))
}

pub async fn admin_update(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    client: Client,
    Path(id): Path<Uuid>,
    Json(b): Json<ConnectorBody>,
) -> ApiResult<StatusCode> {
    load(&state, id).await?;
    let base = b.base_url.as_deref().map(check_url).transpose()?;
    sqlx::query(
        "UPDATE connectors SET name = coalesce($2, name), base_url = coalesce($3, base_url),
                description = CASE WHEN $4::text IS NULL THEN description ELSE nullif($4, '') END, enabled = coalesce($5, enabled), updated_at = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()))
    .bind(&base)
    .bind(b.description.as_deref())
    .bind(b.enabled)
    .execute(&state.db)
    .await?;
    audit::log(
        &state,
        Some(&admin),
        Some(&client),
        "connector.updated",
        Some(("connector", id.to_string())),
        json!({ "name": b.name, "baseUrl": base, "enabled": b.enabled }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn admin_delete(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let name: Option<String> = sqlx::query_scalar("DELETE FROM connectors WHERE id = $1 RETURNING name").bind(id).fetch_optional(&state.db).await?;
    if let Some(name) = name {
        audit::log(&state, Some(&admin), Some(&client), "connector.deleted", Some(("connector", id.to_string())), json!({ "name": name })).await;
    }
    Ok(StatusCode::NO_CONTENT)
}
