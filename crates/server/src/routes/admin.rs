//! Administration (owners and admins) and oversight (also auditors): users, groups, providers,
//! models, settings, the audit log, usage and the web activity log.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::app::{AppState, new_id};
use crate::audit;
use crate::auth::{AdminUser, AuditorUser, Client, hash_password};
use crate::error::{ApiError, ApiResult};
use crate::routes::auth::{validate_email, validate_password};

// ---- users -------------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct UserQuery {
    q: Option<String>,
}

pub async fn list_users(State(state): State<AppState>, AuditorUser(_): AuditorUser, Query(q): Query<UserQuery>) -> ApiResult<Json<Value>> {
    let pattern = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| format!("%{s}%"));
    let rows: Vec<(Uuid, Option<String>, String, String, String, Option<f64>, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>, f64, i64, Vec<String>, bool)> =
        sqlx::query_as(
            "SELECT u.id, u.email, u.name, u.role, u.status, u.monthly_budget_usd, u.last_login_at, u.last_active_at,
                    coalesce((SELECT sum(cost_usd) FROM usage_events e WHERE e.user_id = u.id AND e.created_at >= date_trunc('month', now())), 0)::float8,
                    (SELECT count(*) FROM usage_events e WHERE e.user_id = u.id AND e.created_at >= date_trunc('month', now())),
                    coalesce((SELECT array_agg(g.name ORDER BY g.name) FROM groups g JOIN group_members gm ON gm.group_id = g.id WHERE gm.user_id = u.id), '{}'),
                    u.password_hash IS NOT NULL
               FROM users u
              WHERE $1::text IS NULL OR u.email ILIKE $1 OR u.name ILIKE $1
              ORDER BY u.name",
        )
        .bind(pattern)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|(id, email, name, role, status, budget, login, active, spent, requests, groups, local)| json!({
                "id": id, "email": email, "name": name, "role": role, "status": status, "monthlyBudgetUsd": budget,
                "lastLoginAt": login, "lastActiveAt": active, "spentThisMonth": spent, "requestsThisMonth": requests, "groups": groups, "localPassword": local,
            }))
            .collect::<Vec<_>>()
    )))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUser {
    email: String,
    name: String,
    password: Option<String>,
    role: Option<String>,
}

fn check_role(actor: &crate::auth::User, role: &str) -> ApiResult<()> {
    if !matches!(role, "owner" | "admin" | "auditor" | "member") {
        return Err(ApiError::bad("role must be owner, admin, auditor or member"));
    }
    if role == "owner" && actor.role != "owner" {
        return Err(ApiError::Denied("only owners can make someone an owner".into()));
    }
    Ok(())
}

pub async fn create_user(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Json(req): Json<CreateUser>) -> ApiResult<Json<Value>> {
    let email = validate_email(&req.email)?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad("a name is required"));
    }
    let role = req.role.as_deref().unwrap_or("member");
    check_role(&admin, role)?;
    let hash = match req.password.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => {
            validate_password(p)?;
            Some(hash_password(p.to_string()).await?)
        }
        // SSO-only account (linked at first sign-in by its e-mail)
        None => None,
    };
    let id = new_id();
    sqlx::query("INSERT INTO users (id, email, name, password_hash, role) VALUES ($1, $2, $3, $4, $5)")
        .bind(id)
        .bind(&email)
        .bind(name)
        .bind(hash)
        .bind(role)
        .execute(&state.db)
        .await
        .map_err(|e| match ApiError::from(e) {
            ApiError::Conflict(_) => ApiError::Conflict("someone with this e-mail address exists already".into()),
            other => other,
        })?;
    audit::log(&state, Some(&admin), Some(&client), "user.created", Some(("user", id.to_string())), json!({ "email": email, "role": role })).await;
    Ok(Json(json!({ "id": id })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUser {
    name: Option<String>,
    role: Option<String>,
    status: Option<String>,
    /// a number, or `null` to inherit (group / default budget)
    #[serde(default, with = "nullable")]
    monthly_budget_usd: Option<Option<f64>>,
    password: Option<String>,
}

/// Tells "absent" (`None`) from `null` (`Some(None)`).
mod nullable {
    use serde::{Deserialize, Deserializer};
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<f64>>, D::Error> {
        Ok(Some(Option::<f64>::deserialize(d)?))
    }
}

pub async fn update_user(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Path(id): Path<Uuid>, Json(req): Json<UpdateUser>) -> ApiResult<StatusCode> {
    let target = crate::routes::auth::load_user(&state, id).await?;
    if target.role == "owner" && admin.role != "owner" {
        return Err(ApiError::Denied("only owners can change an owner".into()));
    }
    let mut changes = serde_json::Map::new();
    if let Some(n) = req.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        sqlx::query("UPDATE users SET name = $2 WHERE id = $1").bind(id).bind(n).execute(&state.db).await?;
        changes.insert("name".into(), json!(n));
    }
    if let Some(role) = req.role.as_deref().filter(|r| *r != target.role) {
        check_role(&admin, role)?;
        if target.role == "owner" {
            let owners: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE role = 'owner' AND status = 'active'").fetch_one(&state.db).await?;
            if owners <= 1 {
                return Err(ApiError::bad("there must be at least one owner"));
            }
        }
        sqlx::query("UPDATE users SET role = $2 WHERE id = $1").bind(id).bind(role).execute(&state.db).await?;
        changes.insert("role".into(), json!({ "from": target.role, "to": role }));
    }
    if let Some(status) = req.status.as_deref().filter(|s| *s != target.status) {
        if !matches!(status, "active" | "deactivated") {
            return Err(ApiError::bad("status must be active or deactivated"));
        }
        if id == admin.id {
            return Err(ApiError::bad("you can't deactivate yourself"));
        }
        sqlx::query("UPDATE users SET status = $2 WHERE id = $1").bind(id).bind(status).execute(&state.db).await?;
        if status == "deactivated" {
            sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(id).execute(&state.db).await?;
        }
        changes.insert("status".into(), json!({ "from": target.status, "to": status }));
    }
    if let Some(b) = req.monthly_budget_usd {
        if b.is_some_and(|v| v < 0.0) {
            return Err(ApiError::bad("the budget can't be negative"));
        }
        sqlx::query("UPDATE users SET monthly_budget_usd = $2 WHERE id = $1").bind(id).bind(b).execute(&state.db).await?;
        changes.insert("monthlyBudgetUsd".into(), json!(b));
    }
    if let Some(p) = req.password.as_deref().filter(|p| !p.is_empty()) {
        validate_password(p)?;
        let hash = hash_password(p.to_string()).await?;
        sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1").bind(id).bind(hash).execute(&state.db).await?;
        sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(id).execute(&state.db).await?;
        changes.insert("password".into(), json!("reset"));
    }
    if !changes.is_empty() {
        changes.insert("email".into(), json!(target.email));
        audit::log(&state, Some(&admin), Some(&client), "user.updated", Some(("user", id.to_string())), Value::Object(changes)).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---- groups ------------------------------------------------------------------------------

pub async fn list_groups(State(state): State<AppState>, AuditorUser(_): AuditorUser) -> ApiResult<Json<Value>> {
    let rows: Vec<(Uuid, String, Option<String>, Option<String>, Option<f64>, Vec<Uuid>, f64)> = sqlx::query_as(
        "SELECT g.id, g.name, g.description, g.external_ref, g.monthly_budget_usd,
                coalesce((SELECT array_agg(user_id) FROM group_members WHERE group_id = g.id), '{}'),
                coalesce((SELECT sum(e.cost_usd) FROM usage_events e JOIN group_members gm ON gm.user_id = e.user_id WHERE gm.group_id = g.id AND e.created_at >= date_trunc('month', now())), 0)::float8
           FROM groups g ORDER BY g.name",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|(id, name, description, ext, budget, members, spent)| json!({
                "id": id, "name": name, "description": description, "managedBy": ext.as_deref().and_then(|e| e.split(':').nth(1)), "monthlyBudgetUsd": budget,
                "memberIds": members, "spentThisMonth": spent,
            }))
            .collect::<Vec<_>>()
    )))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupBody {
    name: Option<String>,
    description: Option<String>,
    #[serde(default, with = "nullable")]
    monthly_budget_usd: Option<Option<f64>>,
    member_ids: Option<Vec<Uuid>>,
}

pub async fn create_group(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Json(req): Json<GroupBody>) -> ApiResult<Json<Value>> {
    let name = req.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| ApiError::bad("a name is required"))?;
    let id = new_id();
    sqlx::query("INSERT INTO groups (id, name, description, monthly_budget_usd) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(name)
        .bind(req.description.as_deref())
        .bind(req.monthly_budget_usd.flatten())
        .execute(&state.db)
        .await?;
    if let Some(m) = &req.member_ids {
        set_members(&state, id, m).await?;
    }
    audit::log(&state, Some(&admin), Some(&client), "group.created", Some(("group", id.to_string())), json!({ "name": name, "members": req.member_ids })).await;
    Ok(Json(json!({ "id": id })))
}

async fn set_members(state: &AppState, group: Uuid, members: &[Uuid]) -> ApiResult<()> {
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM group_members WHERE group_id = $1").bind(group).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO group_members (group_id, user_id) SELECT $1, u FROM unnest($2::uuid[]) u ON CONFLICT DO NOTHING").bind(group).bind(members).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn update_group(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Path(id): Path<Uuid>, Json(req): Json<GroupBody>) -> ApiResult<StatusCode> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM groups WHERE id = $1)").bind(id).fetch_one(&state.db).await?;
    if !exists {
        return Err(ApiError::NotFound("group"));
    }
    if let Some(n) = req.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        sqlx::query("UPDATE groups SET name = $2 WHERE id = $1").bind(id).bind(n).execute(&state.db).await?;
    }
    if let Some(d) = &req.description {
        sqlx::query("UPDATE groups SET description = $2 WHERE id = $1").bind(id).bind(d.trim()).execute(&state.db).await?;
    }
    if let Some(b) = req.monthly_budget_usd {
        sqlx::query("UPDATE groups SET monthly_budget_usd = $2 WHERE id = $1").bind(id).bind(b).execute(&state.db).await?;
    }
    if let Some(m) = &req.member_ids {
        set_members(&state, id, m).await?;
    }
    audit::log(
        &state,
        Some(&admin),
        Some(&client),
        "group.updated",
        Some(("group", id.to_string())),
        json!({ "name": req.name, "description": req.description, "monthlyBudgetUsd": req.monthly_budget_usd, "members": req.member_ids }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_group(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let name: Option<String> = sqlx::query_scalar("DELETE FROM groups WHERE id = $1 RETURNING name").bind(id).fetch_optional(&state.db).await?;
    if let Some(name) = name {
        audit::log(&state, Some(&admin), Some(&client), "group.deleted", Some(("group", id.to_string())), json!({ "name": name })).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---- providers ---------------------------------------------------------------------------

pub async fn list_providers(State(state): State<AppState>, AdminUser(_): AdminUser) -> ApiResult<Json<Value>> {
    let rows: Vec<(Uuid, String, String, String, Option<String>, Value, bool, i64)> = sqlx::query_as(
        "SELECT p.id, p.name, p.kind, p.base_url, p.api_key_hint, p.headers, p.enabled, (SELECT count(*) FROM models m WHERE m.provider_id = p.id)
           FROM providers p ORDER BY p.name",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|(id, name, kind, base_url, hint, headers, enabled, models)| json!({
                "id": id, "name": name, "kind": kind, "baseUrl": base_url, "apiKeyHint": hint,
                // header names only: values may be secrets
                "headerNames": headers.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default(),
                "enabled": enabled, "models": models,
            }))
            .collect::<Vec<_>>()
    )))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderBody {
    name: Option<String>,
    kind: Option<String>,
    base_url: Option<String>,
    /// write-only; empty string removes the key
    api_key: Option<String>,
    /// replaces all extra headers
    headers: Option<Value>,
    enabled: Option<bool>,
}

fn check_url(u: &str) -> ApiResult<String> {
    let u = u.trim().trim_end_matches('/').to_string();
    let parsed = url::Url::parse(&u).map_err(|_| ApiError::bad("the base URL isn't valid"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ApiError::bad("the base URL must start with http:// or https://"));
    }
    Ok(u)
}

pub async fn create_provider(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Json(req): Json<ProviderBody>) -> ApiResult<Json<Value>> {
    let kind = req.kind.as_deref().and_then(crate::llm::Kind::parse).ok_or_else(|| ApiError::bad("kind must be anthropic or openai"))?;
    let name = req.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(match kind {
        crate::llm::Kind::Anthropic => "Anthropic",
        crate::llm::Kind::Openai => "OpenAI-compatible",
    });
    let base = match (kind, req.base_url.as_deref().filter(|u| !u.trim().is_empty())) {
        (_, Some(u)) => check_url(u)?,
        (crate::llm::Kind::Anthropic, None) => crate::llm::anthropic::DEFAULT_URL.to_string(),
        (crate::llm::Kind::Openai, None) => return Err(ApiError::bad("an OpenAI-compatible provider needs a base URL (e.g. https://api.openai.com/v1)")),
    };
    let key = req.api_key.as_deref().map(str::trim).filter(|k| !k.is_empty());
    if kind == crate::llm::Kind::Anthropic && key.is_none() {
        return Err(ApiError::bad("Claude needs an Anthropic API key"));
    }
    let enc = key.map(|k| state.secrets.encrypt(k)).transpose()?;
    let id = new_id();
    sqlx::query("INSERT INTO providers (id, name, kind, base_url, api_key_enc, api_key_hint, headers, enabled) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
        .bind(id)
        .bind(name)
        .bind(req.kind.as_deref())
        .bind(&base)
        .bind(enc)
        .bind(key.map(crate::crypto::hint))
        .bind(req.headers.clone().filter(Value::is_object).unwrap_or_else(|| json!({})))
        .bind(req.enabled.unwrap_or(true))
        .execute(&state.db)
        .await?;
    audit::log(&state, Some(&admin), Some(&client), "provider.created", Some(("provider", id.to_string())), json!({ "name": name, "kind": req.kind, "baseUrl": base, "apiKey": key.is_some() })).await;
    Ok(Json(json!({ "id": id })))
}

pub async fn update_provider(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Path(id): Path<Uuid>, Json(req): Json<ProviderBody>) -> ApiResult<StatusCode> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM providers WHERE id = $1)").bind(id).fetch_one(&state.db).await?;
    if !exists {
        return Err(ApiError::NotFound("provider"));
    }
    let mut changed = serde_json::Map::new();
    if let Some(n) = req.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        sqlx::query("UPDATE providers SET name = $2 WHERE id = $1").bind(id).bind(n).execute(&state.db).await?;
        changed.insert("name".into(), json!(n));
    }
    if let Some(u) = &req.base_url {
        let u = check_url(u)?;
        sqlx::query("UPDATE providers SET base_url = $2 WHERE id = $1").bind(id).bind(&u).execute(&state.db).await?;
        changed.insert("baseUrl".into(), json!(u));
    }
    if let Some(k) = req.api_key.as_deref().map(str::trim) {
        let enc = (!k.is_empty()).then(|| state.secrets.encrypt(k)).transpose()?;
        sqlx::query("UPDATE providers SET api_key_enc = $2, api_key_hint = $3 WHERE id = $1")
            .bind(id)
            .bind(enc)
            .bind((!k.is_empty()).then(|| crate::crypto::hint(k)))
            .execute(&state.db)
            .await?;
        changed.insert("apiKey".into(), json!(if k.is_empty() { "removed" } else { "changed" }));
    }
    if let Some(h) = req.headers.clone().filter(Value::is_object) {
        changed.insert("headerNames".into(), json!(h.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>())));
        sqlx::query("UPDATE providers SET headers = $2 WHERE id = $1").bind(id).bind(h).execute(&state.db).await?;
    }
    if let Some(e) = req.enabled {
        sqlx::query("UPDATE providers SET enabled = $2 WHERE id = $1").bind(id).bind(e).execute(&state.db).await?;
        changed.insert("enabled".into(), json!(e));
    }
    sqlx::query("UPDATE providers SET updated_at = now() WHERE id = $1").bind(id).execute(&state.db).await?;
    audit::log(&state, Some(&admin), Some(&client), "provider.updated", Some(("provider", id.to_string())), Value::Object(changed)).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_provider(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let name: Option<String> = sqlx::query_scalar("DELETE FROM providers WHERE id = $1 RETURNING name").bind(id).fetch_optional(&state.db).await?;
    if let Some(name) = name {
        audit::log(&state, Some(&admin), Some(&client), "provider.deleted", Some(("provider", id.to_string())), json!({ "name": name })).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/admin/providers/{id}/discover`: the provider's models (also a connection test).
pub async fn discover(State(state): State<AppState>, AdminUser(_): AdminUser, Path(id): Path<Uuid>) -> ApiResult<Json<Value>> {
    let ep = crate::catalog::endpoint(&state, id).await?;
    let found = crate::llm::list_models(&crate::llm::client(30), &ep).await.map_err(|e| ApiError::bad(format!("the provider answered: {e}")))?;
    let existing: Vec<String> = sqlx::query_scalar("SELECT model FROM models WHERE provider_id = $1").bind(id).fetch_all(&state.db).await?;
    Ok(Json(json!(
        found
            .into_iter()
            .map(|d| json!({ "model": d.model, "displayName": d.display_name, "contextWindow": d.context_window, "maxOutputTokens": d.max_output_tokens, "added": existing.contains(&d.model) }))
            .collect::<Vec<_>>()
    )))
}

// ---- models ------------------------------------------------------------------------------

pub async fn list_models(State(state): State<AppState>, AdminUser(_): AdminUser) -> ApiResult<Json<Value>> {
    let rows = sqlx::query_as::<_, crate::catalog::ModelRow>(sqlx::AssertSqlSafe(format!("{} ORDER BY m.position, m.display_name", crate::catalog::MODEL_SELECT)))
        .fetch_all(&state.db)
        .await?;
    let groups: Vec<(Uuid, Uuid)> = sqlx::query_as("SELECT model_id, group_id FROM model_groups").fetch_all(&state.db).await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|m| {
                let mut v = serde_json::to_value(&m).unwrap_or_default();
                v["groupIds"] = json!(groups.iter().filter(|(mid, _)| *mid == m.id).map(|(_, g)| g).collect::<Vec<_>>());
                v
            })
            .collect::<Vec<_>>()
    )))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelBody {
    provider_id: Option<Uuid>,
    model: Option<String>,
    display_name: Option<String>,
    description: Option<String>,
    enabled: Option<bool>,
    input_price: Option<f64>,
    output_price: Option<f64>,
    cache_read_price: Option<f64>,
    cache_write_price: Option<f64>,
    search_price: Option<f64>,
    #[serde(default, deserialize_with = "opt_i32")]
    context_window: Option<Option<i32>>,
    #[serde(default, deserialize_with = "opt_i32")]
    max_output_tokens: Option<Option<i32>>,
    capabilities: Option<Value>,
    params: Option<Value>,
    access: Option<String>,
    group_ids: Option<Vec<Uuid>>,
    position: Option<i32>,
}

fn opt_i32<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<i32>>, D::Error> {
    Ok(Some(Option::<i32>::deserialize(d)?))
}

pub async fn create_model(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Json(req): Json<ModelBody>) -> ApiResult<Json<Value>> {
    let provider = req.provider_id.ok_or_else(|| ApiError::bad("choose a provider"))?;
    let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM providers WHERE id = $1").bind(provider).fetch_optional(&state.db).await?;
    let kind = kind.ok_or(ApiError::NotFound("provider"))?;
    let model = req.model.as_deref().map(str::trim).filter(|m| !m.is_empty()).ok_or_else(|| ApiError::bad("the model name is required"))?;
    let preset = (kind == "anthropic").then(|| crate::llm::presets::claude(model)).flatten();
    let id = new_id();
    let input = req.input_price.or(preset.map(|p| p.input)).unwrap_or(0.0);
    let search_price = req.search_price.unwrap_or(if kind == "anthropic" { crate::llm::presets::CLAUDE_SEARCH_PRICE } else { 0.0 });
    let caps = req.capabilities.clone().filter(Value::is_object).unwrap_or_else(|| {
        if kind == "anthropic" {
            json!({ "vision": true, "tools": true, "reasoning": crate::llm::presets::claude_thinks(model) })
        } else {
            json!({ "tools": true })
        }
    });
    sqlx::query(
        "INSERT INTO models (id, provider_id, model, display_name, description, enabled, input_price, output_price, cache_read_price, cache_write_price,
                             search_price, context_window, max_output_tokens, capabilities, params, access, position)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
    )
    .bind(id)
    .bind(provider)
    .bind(model)
    .bind(req.display_name.as_deref().map(str::trim).filter(|n| !n.is_empty()).or(preset.map(|p| p.name)).unwrap_or(model))
    .bind(req.description.as_deref())
    .bind(req.enabled.unwrap_or(true))
    .bind(input)
    .bind(req.output_price.or(preset.map(|p| p.output)).unwrap_or(0.0))
    .bind(req.cache_read_price.or(preset.map(|p| p.cache_read)).unwrap_or(0.0))
    .bind(req.cache_write_price.unwrap_or(if preset.is_some() { crate::llm::presets::cache_write_price(input) } else { 0.0 }))
    .bind(search_price)
    .bind(req.context_window.flatten().or(preset.map(|p| p.context)))
    .bind(req.max_output_tokens.flatten())
    .bind(caps)
    .bind(req.params.clone().filter(Value::is_object).unwrap_or_else(|| json!({})))
    .bind(req.access.as_deref().filter(|a| *a == "groups").unwrap_or("all"))
    .bind(req.position.unwrap_or(0))
    .execute(&state.db)
    .await
    .map_err(|e| match ApiError::from(e) {
        ApiError::Conflict(_) => ApiError::Conflict("this provider has that model already".into()),
        other => other,
    })?;
    if let Some(g) = &req.group_ids {
        set_model_groups(&state, id, g).await?;
    }
    audit::log(&state, Some(&admin), Some(&client), "model.created", Some(("model", id.to_string())), json!({ "model": model, "providerId": provider, "access": req.access })).await;
    Ok(Json(json!({ "id": id })))
}

async fn set_model_groups(state: &AppState, model: Uuid, groups: &[Uuid]) -> ApiResult<()> {
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM model_groups WHERE model_id = $1").bind(model).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO model_groups (model_id, group_id) SELECT $1, g FROM unnest($2::uuid[]) g").bind(model).bind(groups).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn update_model(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Path(id): Path<Uuid>, Json(req): Json<ModelBody>) -> ApiResult<StatusCode> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM models WHERE id = $1)").bind(id).fetch_one(&state.db).await?;
    if !exists {
        return Err(ApiError::NotFound("model"));
    }
    if req.access.as_deref().is_some_and(|a| !matches!(a, "all" | "groups")) {
        return Err(ApiError::bad("access must be all or groups"));
    }
    if [req.input_price, req.output_price, req.cache_read_price, req.cache_write_price, req.search_price].iter().flatten().any(|p| *p < 0.0) {
        return Err(ApiError::bad("prices can't be negative"));
    }
    sqlx::query(
        "UPDATE models SET
            model = coalesce($2, model), display_name = coalesce($3, display_name), description = CASE WHEN $4::text IS NULL THEN description ELSE nullif($4, '') END,
            enabled = coalesce($5, enabled), input_price = coalesce($6, input_price), output_price = coalesce($7, output_price),
            cache_read_price = coalesce($8, cache_read_price), cache_write_price = coalesce($9, cache_write_price), search_price = coalesce($10, search_price),
            context_window = CASE WHEN $11 THEN $12 ELSE context_window END, max_output_tokens = CASE WHEN $13 THEN $14 ELSE max_output_tokens END,
            capabilities = coalesce($15, capabilities), params = coalesce($16, params), access = coalesce($17, access), position = coalesce($18, position),
            updated_at = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(req.model.as_deref().map(str::trim).filter(|m| !m.is_empty()))
    .bind(req.display_name.as_deref().map(str::trim).filter(|m| !m.is_empty()))
    .bind(req.description.as_deref())
    .bind(req.enabled)
    .bind(req.input_price)
    .bind(req.output_price)
    .bind(req.cache_read_price)
    .bind(req.cache_write_price)
    .bind(req.search_price)
    .bind(req.context_window.is_some())
    .bind(req.context_window.flatten())
    .bind(req.max_output_tokens.is_some())
    .bind(req.max_output_tokens.flatten())
    .bind(req.capabilities.clone().filter(Value::is_object))
    .bind(req.params.clone().filter(Value::is_object))
    .bind(req.access.as_deref())
    .bind(req.position)
    .execute(&state.db)
    .await?;
    if let Some(g) = &req.group_ids {
        set_model_groups(&state, id, g).await?;
    }
    audit::log(
        &state,
        Some(&admin),
        Some(&client),
        "model.updated",
        Some(("model", id.to_string())),
        json!({
            "enabled": req.enabled, "access": req.access, "groupIds": req.group_ids, "inputPrice": req.input_price, "outputPrice": req.output_price,
            "displayName": req.display_name, "params": req.params, "capabilities": req.capabilities,
        }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_model(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let model: Option<String> = sqlx::query_scalar("DELETE FROM models WHERE id = $1 RETURNING model").bind(id).fetch_optional(&state.db).await?;
    if let Some(model) = model {
        audit::log(&state, Some(&admin), Some(&client), "model.deleted", Some(("model", id.to_string())), json!({ "model": model })).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---- settings ----------------------------------------------------------------------------

fn settings_json(s: &crate::settings::Settings) -> Value {
    let mut v = serde_json::to_value(s).unwrap_or_default();
    v["webSearch"]["apiKeySet"] = json!(s.web_search.api_key_enc.is_some());
    v
}

pub async fn get_settings(State(state): State<AppState>, AdminUser(_): AdminUser) -> ApiResult<Json<Value>> {
    let s = crate::settings::load(&state).await?;
    let mut v = settings_json(&s);
    v["authProviders"] = json!(state.providers.iter().map(|p| json!({ "id": p.id(), "name": p.display_name(), "type": p.kind() })).collect::<Vec<_>>());
    Ok(Json(v))
}

pub async fn put_settings(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Json(mut body): Json<Value>) -> ApiResult<Json<Value>> {
    let old = crate::settings::load(&state).await?;
    // the search API key is write-only: `apiKey` sets it, "" removes it, absent keeps it
    let new_key = body["webSearch"].get("apiKey").and_then(Value::as_str).map(str::trim).map(String::from);
    if let Some(ws) = body.get_mut("webSearch").and_then(Value::as_object_mut) {
        ws.remove("apiKey");
        ws.remove("apiKeySet");
        ws.remove("apiKeyEnc");
    }
    let mut s: crate::settings::Settings = serde_json::from_value(body).map_err(|e| ApiError::bad(format!("invalid settings: {e}")))?;
    s.web_search.api_key_enc = match new_key {
        Some(k) if k.is_empty() => None,
        Some(k) => {
            use base64::Engine;
            Some(base64::engine::general_purpose::STANDARD.encode(state.secrets.encrypt(&k)?))
        }
        None => old.web_search.api_key_enc.clone(),
    };
    if !matches!(s.audit_content.as_str(), "full" | "metadata") {
        return Err(ApiError::bad("auditContent must be full or metadata"));
    }
    if !matches!(s.web_search.engine.as_str(), "none" | "searxng" | "brave" | "tavily") {
        return Err(ApiError::bad("the web search engine must be none, searxng, brave or tavily"));
    }
    s.web_search.max_results = s.web_search.max_results.clamp(1, 20);
    s.web_search.max_tool_calls = s.web_search.max_tool_calls.clamp(1, 30);
    crate::settings::save(&state, &s).await?;
    // what changed, for the audit log (secrets as a flag only)
    let (a, b) = (settings_json(&old), settings_json(&s));
    let mut diff = serde_json::Map::new();
    for (k, v) in b.as_object().into_iter().flatten() {
        if a.get(k) != Some(v) {
            diff.insert(k.clone(), json!({ "from": a.get(k), "to": v }));
        }
    }
    if !diff.is_empty() {
        audit::log(&state, Some(&admin), Some(&client), "settings.updated", None, Value::Object(diff)).await;
    }
    Ok(Json(settings_json(&s)))
}

// ---- audit log ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuditQuery {
    actor_id: Option<Uuid>,
    /// prefix, e.g. `chat.` or `auth.login`
    action: Option<String>,
    q: Option<String>,
    from: Option<chrono::NaiveDate>,
    to: Option<chrono::NaiveDate>,
    before_id: Option<i64>,
    limit: Option<i64>,
    format: Option<String>,
}

type AuditRow = (i64, chrono::DateTime<chrono::Utc>, Option<Uuid>, Option<String>, Option<String>, Option<String>, String, Option<String>, Option<String>, Value, Vec<u8>);

async fn audit_rows(state: &AppState, q: &AuditQuery, limit: i64) -> ApiResult<Vec<AuditRow>> {
    let pattern = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| format!("%{s}%"));
    Ok(sqlx::query_as(
        "SELECT id, at, actor_id, actor_label, ip, user_agent, action, target_type, target_id, data, hash FROM audit_events
          WHERE ($1::uuid IS NULL OR actor_id = $1)
            AND ($2::text IS NULL OR starts_with(action, $2))
            AND ($3::text IS NULL OR data::text ILIKE $3 OR actor_label ILIKE $3 OR target_id ILIKE $3 OR ip ILIKE $3)
            AND ($4::date IS NULL OR at >= $4::date)
            AND ($5::date IS NULL OR at < $5::date + 1)
            AND ($6::int8 IS NULL OR id < $6)
          ORDER BY id DESC LIMIT $7",
    )
    .bind(q.actor_id)
    .bind(q.action.as_deref().filter(|a| !a.is_empty()))
    .bind(pattern)
    .bind(q.from)
    .bind(q.to)
    .bind(q.before_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await?)
}

fn audit_json(r: &AuditRow) -> Value {
    json!({
        "id": r.0, "at": r.1, "actorId": r.2, "actor": r.3, "ip": r.4, "userAgent": r.5, "action": r.6,
        "targetType": r.7, "targetId": r.8, "data": r.9, "hash": hex::encode(&r.10),
    })
}

pub async fn audit_log(State(state): State<AppState>, AuditorUser(_): AuditorUser, Query(q): Query<AuditQuery>) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let rows = audit_rows(&state, &q, limit).await?;
    Ok(Json(json!({ "events": rows.iter().map(audit_json).collect::<Vec<_>>(), "more": rows.len() as i64 == limit })))
}

pub async fn audit_verify(State(state): State<AppState>, AuditorUser(user): AuditorUser, client: Client) -> ApiResult<Json<Value>> {
    let v = audit::verify(&state).await?;
    audit::log(&state, Some(&user), Some(&client), "audit.verified", None, json!({ "ok": v.ok, "checked": v.checked, "firstBadId": v.first_bad_id })).await;
    Ok(Json(serde_json::to_value(v).unwrap_or_default()))
}

/// `GET /api/admin/audit/export?format=csv|jsonl` (same filters as the list, up to 100,000 rows).
pub async fn audit_export(State(state): State<AppState>, AuditorUser(user): AuditorUser, client: Client, Query(q): Query<AuditQuery>) -> ApiResult<Response> {
    let rows = audit_rows(&state, &q, 100_000).await?;
    let format = q.format.as_deref().unwrap_or("jsonl");
    audit::log(&state, Some(&user), Some(&client), "audit.exported", None, json!({ "format": format, "rows": rows.len(), "action": q.action, "actorId": q.actor_id, "from": q.from, "to": q.to })).await;
    let (body, ctype, ext) = if format == "csv" {
        let mut out = String::from("id,at,actor,ip,action,target_type,target_id,data,hash\n");
        for r in rows.iter().rev() {
            let fields = [r.0.to_string(), r.1.to_rfc3339(), r.3.clone().unwrap_or_default(), r.4.clone().unwrap_or_default(), r.6.clone(), r.7.clone().unwrap_or_default(), r.8.clone().unwrap_or_default(), r.9.to_string(), hex::encode(&r.10)];
            out.push_str(&fields.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(","));
            out.push('\n');
        }
        (out, "text/csv; charset=utf-8", "csv")
    } else {
        (rows.iter().rev().map(|r| audit_json(r).to_string() + "\n").collect::<String>(), "application/x-ndjson", "jsonl")
    };
    let name = format!("agora-audit-{}.{ext}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    Ok(([(header::CONTENT_TYPE, ctype.to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))], body).into_response())
}

pub fn csv_field(s: &str) -> String {
    // also defuses spreadsheet formulas
    let s = if s.starts_with(['=', '+', '-', '@']) { format!("'{s}") } else { s.to_string() };
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
}

// ---- usage -------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageQuery {
    from: Option<chrono::NaiveDate>,
    to: Option<chrono::NaiveDate>,
    user_id: Option<Uuid>,
    format: Option<String>,
}

pub async fn usage_summary(State(state): State<AppState>, AuditorUser(_): AuditorUser, Query(q): Query<UsageQuery>) -> ApiResult<Json<Value>> {
    let from = q.from.unwrap_or_else(|| (chrono::Utc::now() - chrono::Duration::days(29)).date_naive());
    let to = q.to.unwrap_or_else(|| chrono::Utc::now().date_naive());
    const WHERE: &str = "e.created_at >= $1::date AND e.created_at < $2::date + 1 AND ($3::uuid IS NULL OR e.user_id = $3)";
    let totals: (i64, i64, i64, i64, i64, f64, i64) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT count(*), coalesce(sum(input_tokens), 0)::int8, coalesce(sum(output_tokens), 0)::int8, coalesce(sum(cache_read_tokens + cache_write_tokens), 0)::int8,
                count(*) FILTER (WHERE status = 'error'), coalesce(sum(cost_usd), 0)::float8, count(DISTINCT user_id)
           FROM usage_events e WHERE {WHERE}"
    )))
    .bind(from)
    .bind(to)
    .bind(q.user_id)
    .fetch_one(&state.db)
    .await?;
    let days: Vec<(chrono::NaiveDate, i64, f64, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT d::date, count(e.id), coalesce(sum(e.cost_usd), 0)::float8, coalesce(sum(e.input_tokens + e.output_tokens + e.cache_read_tokens + e.cache_write_tokens), 0)::int8
           FROM generate_series($1::date, $2::date, interval '1 day') d
           LEFT JOIN usage_events e ON e.created_at >= d AND e.created_at < d + interval '1 day' AND ($3::uuid IS NULL OR e.user_id = $3)
          GROUP BY d ORDER BY d"
    )))
    .bind(from)
    .bind(to)
    .bind(q.user_id)
    .fetch_all(&state.db)
    .await?;
    let users: Vec<(Option<Uuid>, Option<String>, Option<String>, i64, i64, i64, f64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT e.user_id, u.name, u.email, count(*), sum(e.input_tokens + e.cache_read_tokens + e.cache_write_tokens)::int8, sum(e.output_tokens)::int8, sum(e.cost_usd)::float8
           FROM usage_events e LEFT JOIN users u ON u.id = e.user_id WHERE {WHERE}
          GROUP BY e.user_id, u.name, u.email ORDER BY 7 DESC LIMIT 200"
    )))
    .bind(from)
    .bind(to)
    .bind(q.user_id)
    .fetch_all(&state.db)
    .await?;
    let models: Vec<(String, i64, i64, i64, f64, Option<f64>)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT e.model_name, count(*), sum(e.input_tokens + e.cache_read_tokens + e.cache_write_tokens)::int8, sum(e.output_tokens)::int8, sum(e.cost_usd)::float8,
                avg(e.latency_ms)::float8
           FROM usage_events e WHERE {WHERE} GROUP BY e.model_name ORDER BY 5 DESC"
    )))
    .bind(from)
    .bind(to)
    .bind(q.user_id)
    .fetch_all(&state.db)
    .await?;
    let groups: Vec<(String, i64, f64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT g.name, count(e.id), coalesce(sum(e.cost_usd), 0)::float8
           FROM groups g JOIN group_members gm ON gm.group_id = g.id JOIN usage_events e ON e.user_id = gm.user_id
          WHERE {WHERE} GROUP BY g.name ORDER BY 3 DESC"
    )))
    .bind(from)
    .bind(to)
    .bind(q.user_id)
    .fetch_all(&state.db)
    .await?;
    let searches: (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE kind = 'search'), count(*) FILTER (WHERE kind = 'fetch') FROM web_events e
          WHERE e.created_at >= $1::date AND e.created_at < $2::date + 1 AND ($3::uuid IS NULL OR e.user_id = $3)",
    )
    .bind(from)
    .bind(to)
    .bind(q.user_id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(json!({
        "from": from, "to": to,
        "totals": { "requests": totals.0, "inputTokens": totals.1, "outputTokens": totals.2, "cacheTokens": totals.3, "errors": totals.4, "costUsd": totals.5,
                    "users": totals.6, "searches": searches.0, "fetches": searches.1 },
        "days": days.iter().map(|(d, n, c, t)| json!({ "day": d, "requests": n, "costUsd": c, "tokens": t })).collect::<Vec<_>>(),
        "users": users.iter().map(|(id, name, email, n, i, o, c)| json!({ "userId": id, "name": name, "email": email, "requests": n, "inputTokens": i, "outputTokens": o, "costUsd": c })).collect::<Vec<_>>(),
        "models": models.iter().map(|(m, n, i, o, c, l)| json!({ "model": m, "requests": n, "inputTokens": i, "outputTokens": o, "costUsd": c, "avgLatencyMs": l })).collect::<Vec<_>>(),
        "groups": groups.iter().map(|(g, n, c)| json!({ "group": g, "requests": n, "costUsd": c })).collect::<Vec<_>>(),
    })))
}

/// `GET /api/admin/usage/export`: every usage event in the range as CSV.
pub async fn usage_export(State(state): State<AppState>, AuditorUser(user): AuditorUser, client: Client, Query(q): Query<UsageQuery>) -> ApiResult<Response> {
    let from = q.from.unwrap_or_else(|| (chrono::Utc::now() - chrono::Duration::days(29)).date_naive());
    let to = q.to.unwrap_or_else(|| chrono::Utc::now().date_naive());
    let rows: Vec<(chrono::DateTime<chrono::Utc>, Option<String>, String, String, i64, i64, i64, i64, i32, f64, Option<i32>, String, Option<Uuid>)> = sqlx::query_as(
        "SELECT e.created_at, u.email, e.model_name, e.source, e.input_tokens, e.output_tokens, e.cache_read_tokens, e.cache_write_tokens, e.provider_searches,
                e.cost_usd, e.latency_ms, e.status, e.chat_id
           FROM usage_events e LEFT JOIN users u ON u.id = e.user_id
          WHERE e.created_at >= $1::date AND e.created_at < $2::date + 1 AND ($3::uuid IS NULL OR e.user_id = $3) ORDER BY e.created_at",
    )
    .bind(from)
    .bind(to)
    .bind(q.user_id)
    .fetch_all(&state.db)
    .await?;
    audit::log(&state, Some(&user), Some(&client), "usage.exported", None, json!({ "from": from, "to": to, "rows": rows.len(), "userId": q.user_id })).await;
    let _ = q.format;
    let mut out = String::from("time,user,model,source,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,provider_searches,cost_usd,latency_ms,status,chat_id\n");
    for r in rows {
        let f = [
            r.0.to_rfc3339(),
            r.1.unwrap_or_default(),
            r.2,
            r.3,
            r.4.to_string(),
            r.5.to_string(),
            r.6.to_string(),
            r.7.to_string(),
            r.8.to_string(),
            format!("{:.6}", r.9),
            r.10.map(|l| l.to_string()).unwrap_or_default(),
            r.11,
            r.12.map(|c| c.to_string()).unwrap_or_default(),
        ];
        out.push_str(&f.iter().map(|x| csv_field(x)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    let name = format!("agora-usage-{from}-{to}.csv");
    Ok(([(header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))], out).into_response())
}

// ---- web activity ------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebQuery {
    user_id: Option<Uuid>,
    kind: Option<String>,
    q: Option<String>,
    before: Option<chrono::DateTime<chrono::Utc>>,
    limit: Option<i64>,
}

pub async fn web_events(State(state): State<AppState>, AuditorUser(_): AuditorUser, Query(q): Query<WebQuery>) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let pattern = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| format!("%{s}%"));
    let rows: Vec<(Uuid, chrono::DateTime<chrono::Utc>, Option<Uuid>, Option<String>, Option<Uuid>, String, String, Option<String>, Option<String>, Value, i32, String, Option<String>, Option<i32>)> =
        sqlx::query_as(
            "SELECT w.id, w.created_at, w.user_id, u.email, w.chat_id, w.kind, w.engine, w.query, w.url, w.results, w.result_count, w.status, w.error, w.duration_ms
               FROM web_events w LEFT JOIN users u ON u.id = w.user_id
              WHERE ($1::uuid IS NULL OR w.user_id = $1) AND ($2::text IS NULL OR w.kind = $2)
                AND ($3::text IS NULL OR w.query ILIKE $3 OR w.url ILIKE $3 OR w.results::text ILIKE $3)
                AND ($4::timestamptz IS NULL OR w.created_at < $4)
              ORDER BY w.created_at DESC LIMIT $5",
        )
        .bind(q.user_id)
        .bind(q.kind.as_deref().filter(|k| !k.is_empty()))
        .bind(pattern)
        .bind(q.before)
        .bind(limit)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(json!({
        "events": rows.iter().map(|r| json!({
            "id": r.0, "at": r.1, "userId": r.2, "user": r.3, "chatId": r.4, "kind": r.5, "engine": r.6, "query": r.7, "url": r.8,
            "results": r.9, "resultCount": r.10, "status": r.11, "error": r.12, "durationMs": r.13,
        })).collect::<Vec<_>>(),
        "more": rows.len() as i64 == limit,
    })))
}

// ---- conversation inspector (auditors) ---------------------------------------------------

pub async fn user_chats(State(state): State<AppState>, AuditorUser(_): AuditorUser, Path(id): Path<Uuid>) -> ApiResult<Json<Value>> {
    let rows: Vec<(Uuid, String, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>, Option<chrono::DateTime<chrono::Utc>>, i64, f64)> = sqlx::query_as(
        "SELECT c.id, c.title, c.created_at, c.updated_at, c.deleted_at, (SELECT count(*) FROM messages m WHERE m.chat_id = c.id),
                coalesce((SELECT sum(cost_usd) FROM messages m WHERE m.chat_id = c.id), 0)::float8
           FROM chats c WHERE c.user_id = $1 ORDER BY c.updated_at DESC LIMIT 1000",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(
        rows.iter()
            .map(|(id, t, c, u, d, n, cost)| json!({ "id": id, "title": t, "createdAt": c, "updatedAt": u, "deletedAt": d, "messages": n, "costUsd": cost }))
            .collect::<Vec<_>>()
    )))
}

/// Reading someone's conversation is itself audited.
pub async fn inspect_chat(State(state): State<AppState>, AuditorUser(auditor): AuditorUser, client: Client, Path(id): Path<Uuid>) -> ApiResult<Json<Value>> {
    let chat = crate::routes::chats::load_chat(&state, id).await?;
    let messages = crate::routes::chats::load_messages(&state, id).await?;
    let owner = crate::routes::auth::load_user(&state, chat.user_id).await.ok();
    audit::log(
        &state,
        Some(&auditor),
        Some(&client),
        "audit.chat_viewed",
        Some(("chat", id.to_string())),
        json!({ "ownerId": chat.user_id, "owner": owner.as_ref().map(|o| o.label()), "title": chat.title }),
    )
    .await;
    let web: Vec<(chrono::DateTime<chrono::Utc>, String, Option<String>, Option<String>, String)> =
        sqlx::query_as("SELECT created_at, kind, query, url, status FROM web_events WHERE chat_id = $1 ORDER BY created_at").bind(id).fetch_all(&state.db).await?;
    Ok(Json(json!({
        "chat": chat, "owner": owner, "messages": messages,
        "web": web.iter().map(|(at, k, q, u, s)| json!({ "at": at, "kind": k, "query": q, "url": u, "status": s })).collect::<Vec<_>>(),
    })))
}

/// `GET /api/admin/overview`: the dashboard's numbers.
pub async fn overview(State(state): State<AppState>, AuditorUser(_): AuditorUser) -> ApiResult<Json<Value>> {
    let (users, active7, chats_today, cost_month, req_today): (i64, i64, i64, f64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM users WHERE status = 'active'),
                (SELECT count(*) FROM users WHERE last_active_at > now() - interval '7 days'),
                (SELECT count(*) FROM chats WHERE created_at >= current_date),
                (SELECT coalesce(sum(cost_usd), 0)::float8 FROM usage_events WHERE created_at >= date_trunc('month', now())),
                (SELECT count(*) FROM usage_events WHERE created_at >= current_date)",
    )
    .fetch_one(&state.db)
    .await?;
    let (providers, models): (i64, i64) =
        sqlx::query_as("SELECT (SELECT count(*) FROM providers WHERE enabled), (SELECT count(*) FROM models WHERE enabled)").fetch_one(&state.db).await?;
    let s = crate::settings::load(&state).await?;
    Ok(Json(json!({
        "users": users, "activeLast7Days": active7, "chatsToday": chats_today, "costThisMonth": cost_month, "requestsToday": req_today,
        "providers": providers, "models": models, "webSearch": s.web_search.engine, "claudeNativeSearch": s.web_search.claude_native,
        "secretKeySet": !state.config.secret_key.trim().is_empty(),
    })))
}
