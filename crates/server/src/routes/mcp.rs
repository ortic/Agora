//! MCP servers of the organization: what people may use, confirming a call in the chat, and
//! administration (servers, their tools, who gets them).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::app::{AppState, new_id};
use crate::audit;
use crate::auth::{AdminUser, Client, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::mcp::{self, Secrets};

/// `GET /api/mcp-servers`: the servers the person may use (for the switch in the chat).
pub async fn mine(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<Value>> {
    let defs = mcp::granted(&state, user.id).await?;
    Ok(Json(json!(defs.iter().map(|d| json!({ "id": d.id, "name": d.name, "description": d.description, "approval": d.approval })).collect::<Vec<_>>())))
}

#[derive(Deserialize)]
pub struct Decision {
    allow: bool,
}

/// `POST /api/tool-calls/{id}/decision`: the person allows or declines a call waiting for them.
pub async fn decide(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    client: Client,
    Path(id): Path<String>,
    Json(req): Json<Decision>,
) -> ApiResult<StatusCode> {
    let pending = state.approvals.remove_if(&id, |_, p| p.user_id == user.id).map(|(_, p)| p);
    let Some(p) = pending else {
        // already decided, ended, or on another replica
        return Err(ApiError::NotFound("call waiting for a decision"));
    };
    let _ = p.tx.send(req.allow);
    audit::log(&state, Some(&user), Some(&client), "mcp.decision", Some(("tool_call", id)), json!({ "allow": req.allow })).await;
    Ok(StatusCode::NO_CONTENT)
}

// ---- administration ------------------------------------------------------------------------

type Row = (Uuid, String, String, Option<String>, Value, Option<Vec<u8>>, Vec<String>, String, bool, String, Vec<Uuid>);

/// `GET /api/admin/mcp-servers`: the servers, without their secrets (only which there are).
pub async fn admin_list(State(state): State<AppState>, AdminUser(_): AdminUser) -> ApiResult<Json<Value>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT s.id, s.name, s.kind, s.description, s.config, s.secrets_enc, s.tools, s.approval, s.enabled, s.access,
                coalesce((SELECT array_agg(group_id) FROM mcp_server_groups g WHERE g.server_id = s.id), '{}')
           FROM mcp_servers s ORDER BY s.name",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|(id, name, kind, description, config, sealed, tools, approval, enabled, access, groups)| {
                let secrets = Secrets::open(&state, sealed.as_deref());
                let keys = |m: &std::collections::HashMap<String, String>| {
                    let mut k: Vec<&String> = m.keys().collect();
                    k.sort();
                    k.into_iter().cloned().collect::<Vec<_>>()
                };
                json!({
                    "id": id, "name": name, "kind": kind, "description": description, "config": config,
                    "envKeys": keys(&secrets.env), "headerKeys": keys(&secrets.headers),
                    "tools": tools, "approval": approval, "enabled": enabled, "access": access, "groupIds": groups,
                })
            })
            .collect::<Vec<_>>()
    )))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerBody {
    /// for a check: the server whose stored secrets fill in what isn't given
    id: Option<Uuid>,
    name: Option<String>,
    kind: Option<String>,
    description: Option<String>,
    config: Option<Value>,
    /// replaces the stored environment (absent: kept)
    env: Option<std::collections::HashMap<String, String>>,
    /// replaces the stored headers (absent: kept)
    headers: Option<std::collections::HashMap<String, String>>,
    tools: Option<Vec<String>>,
    approval: Option<String>,
    enabled: Option<bool>,
    access: Option<String>,
    group_ids: Option<Vec<Uuid>>,
}

fn choice(what: &str, value: Option<&str>, allowed: &[&str]) -> ApiResult<()> {
    match value {
        Some(v) if !allowed.contains(&v) => Err(ApiError::bad(format!("{what} is one of: {}", allowed.join(", ")))),
        _ => Ok(()),
    }
}

fn clean_pairs(m: &std::collections::HashMap<String, String>) -> ApiResult<std::collections::HashMap<String, String>> {
    let mut out = std::collections::HashMap::new();
    for (k, v) in m {
        let k = k.trim();
        if k.is_empty() {
            continue;
        }
        if k.contains(['=', '\0', '\n', ':']) {
            return Err(ApiError::bad(format!("“{k}” isn't a valid name")));
        }
        out.insert(k.to_string(), v.trim().to_string());
    }
    Ok(out)
}

/// The secrets after a change: given parts replace the stored ones.
async fn merged_secrets(state: &AppState, id: Option<Uuid>, req: &ServerBody) -> ApiResult<Secrets> {
    let mut s = match id {
        Some(id) => {
            let sealed: Option<Option<Vec<u8>>> =
                sqlx::query_scalar("SELECT secrets_enc FROM mcp_servers WHERE id = $1").bind(id).fetch_optional(&state.db).await?;
            Secrets::open(state, sealed.flatten().as_deref())
        }
        None => Secrets::default(),
    };
    if let Some(env) = &req.env {
        s.env = clean_pairs(env)?;
    }
    if let Some(h) = &req.headers {
        s.headers = clean_pairs(h)?;
    }
    Ok(s)
}

fn check_stdio(state: &AppState, kind: &str) -> ApiResult<()> {
    if kind == "mcp_stdio" && !state.config.mcp_stdio {
        return Err(ApiError::bad("starting programs is turned off on this server (AGORA_MCP_STDIO): use a server at a URL"));
    }
    Ok(())
}

async fn set_groups(state: &AppState, id: Uuid, groups: &[Uuid]) -> ApiResult<()> {
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM mcp_server_groups WHERE server_id = $1").bind(id).execute(&mut *tx).await?;
    for g in groups {
        sqlx::query("INSERT INTO mcp_server_groups (server_id, group_id) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(id).bind(g).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn name_taken(state: &AppState, name: &str, except: Option<Uuid>) -> ApiResult<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM mcp_servers WHERE lower(name) = lower($1) AND id IS DISTINCT FROM $2)")
        .bind(name)
        .bind(except)
        .fetch_one(&state.db)
        .await?)
}

fn short(s: &str, n: usize) -> String {
    s.trim().chars().take(n).collect()
}

/// `POST /api/admin/mcp-servers`
pub async fn admin_create(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Json(req): Json<ServerBody>) -> ApiResult<Json<Value>> {
    let name = short(req.name.as_deref().unwrap_or_default(), 80);
    if name.is_empty() {
        return Err(ApiError::bad("give the server a name"));
    }
    if name_taken(&state, &name, None).await? {
        return Err(ApiError::bad("another server has that name"));
    }
    let kind = req.kind.as_deref().unwrap_or("mcp_http");
    check_stdio(&state, kind)?;
    let config = mcp::clean_config(kind, req.config.as_ref().unwrap_or(&Value::Null)).map_err(ApiError::bad)?;
    choice("approval", req.approval.as_deref(), &["ask", "auto"])?;
    choice("access", req.access.as_deref(), &["all", "groups"])?;
    let secrets = merged_secrets(&state, None, &req).await?;
    let id = new_id();
    sqlx::query(
        "INSERT INTO mcp_servers (id, name, kind, description, config, secrets_enc, tools, approval, enabled, access)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(id)
    .bind(&name)
    .bind(kind)
    .bind(req.description.as_deref().map(str::trim).filter(|d| !d.is_empty()))
    .bind(&config)
    .bind(secrets.seal(&state)?)
    .bind(req.tools.clone().unwrap_or_default())
    .bind(req.approval.as_deref().unwrap_or("ask"))
    .bind(req.enabled.unwrap_or(true))
    .bind(req.access.as_deref().unwrap_or("groups"))
    .execute(&state.db)
    .await?;
    if let Some(g) = &req.group_ids {
        set_groups(&state, id, g).await?;
    }
    audit::log(
        &state,
        Some(&admin),
        Some(&client),
        "mcp.server_created",
        Some(("mcp_server", id.to_string())),
        json!({ "name": name, "kind": kind, "config": config, "envKeys": secrets.env.keys().collect::<Vec<_>>(), "headerKeys": secrets.headers.keys().collect::<Vec<_>>(),
                "tools": req.tools, "approval": req.approval, "access": req.access, "groups": req.group_ids }),
    )
    .await;
    Ok(Json(json!({ "id": id })))
}

/// `PATCH /api/admin/mcp-servers/{id}`
pub async fn admin_update(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    client: Client,
    Path(id): Path<Uuid>,
    Json(req): Json<ServerBody>,
) -> ApiResult<StatusCode> {
    let stored: Option<String> = sqlx::query_scalar("SELECT kind FROM mcp_servers WHERE id = $1").bind(id).fetch_optional(&state.db).await?;
    let kind = req.kind.clone().unwrap_or(stored.ok_or(ApiError::NotFound("MCP server"))?);
    check_stdio(&state, &kind)?;
    choice("approval", req.approval.as_deref(), &["ask", "auto"])?;
    choice("access", req.access.as_deref(), &["all", "groups"])?;
    let config = match &req.config {
        Some(c) => Some(mcp::clean_config(&kind, c).map_err(ApiError::bad)?),
        None if req.kind.is_some() => return Err(ApiError::bad("a server of another kind needs its settings")),
        None => None,
    };
    let name = req.name.as_deref().map(|n| short(n, 80)).filter(|n| !n.is_empty());
    if let Some(n) = &name
        && name_taken(&state, n, Some(id)).await?
    {
        return Err(ApiError::bad("another server has that name"));
    }
    let secrets_changed = req.env.is_some() || req.headers.is_some();
    let sealed = if secrets_changed { merged_secrets(&state, Some(id), &req).await?.seal(&state)? } else { None };
    sqlx::query(
        "UPDATE mcp_servers SET name = coalesce($2, name), kind = $3, config = coalesce($4, config), tools = coalesce($5, tools),
                approval = coalesce($6, approval), enabled = coalesce($7, enabled), access = coalesce($8, access),
                description = CASE WHEN $9 THEN $10 ELSE description END,
                secrets_enc = CASE WHEN $11 THEN $12 ELSE secrets_enc END, updated_at = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(&name)
    .bind(&kind)
    .bind(&config)
    .bind(&req.tools)
    .bind(&req.approval)
    .bind(req.enabled)
    .bind(&req.access)
    .bind(req.description.is_some())
    .bind(req.description.as_deref().map(str::trim).filter(|d| !d.is_empty()))
    .bind(secrets_changed)
    .bind(&sealed)
    .execute(&state.db)
    .await?;
    if let Some(g) = &req.group_ids {
        set_groups(&state, id, g).await?;
    }
    state.mcp.drop_server(id);
    audit::log(
        &state,
        Some(&admin),
        Some(&client),
        "mcp.server_updated",
        Some(("mcp_server", id.to_string())),
        json!({ "name": name, "kind": kind, "config": config, "secretsChanged": secrets_changed, "envKeys": req.env.as_ref().map(|e| e.keys().collect::<Vec<_>>()),
                "headerKeys": req.headers.as_ref().map(|h| h.keys().collect::<Vec<_>>()), "tools": req.tools, "approval": req.approval, "enabled": req.enabled,
                "access": req.access, "groups": req.group_ids }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/admin/mcp-servers/{id}`
pub async fn admin_delete(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let name: Option<String> = sqlx::query_scalar("DELETE FROM mcp_servers WHERE id = $1 RETURNING name").bind(id).fetch_optional(&state.db).await?;
    let name = name.ok_or(ApiError::NotFound("MCP server"))?;
    state.mcp.drop_server(id);
    audit::log(&state, Some(&admin), Some(&client), "mcp.server_deleted", Some(("mcp_server", id.to_string())), json!({ "name": name })).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/admin/mcp-servers/check`: starts (or reaches) a server with the given settings
/// and lists its tools, without saving anything. Secrets not given come from the stored server
/// `id`.
pub async fn admin_check(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Json(req): Json<ServerBody>) -> ApiResult<Json<Value>> {
    let kind = req.kind.as_deref().unwrap_or("mcp_http");
    check_stdio(&state, kind)?;
    let config = mcp::clean_config(kind, req.config.as_ref().unwrap_or(&Value::Null)).map_err(ApiError::bad)?;
    let secrets = merged_secrets(&state, req.id, &req).await?;
    let def = mcp::Def {
        id: req.id.unwrap_or_else(new_id),
        name: short(req.name.as_deref().unwrap_or("The server"), 80),
        kind: kind.to_string(),
        description: None,
        config: config.clone(),
        secrets,
        tools: vec![],
        approval: "ask".into(),
        updated_at: chrono::Utc::now(),
    };
    audit::log(
        &state,
        Some(&admin),
        Some(&client),
        "mcp.server_checked",
        req.id.map(|id| ("mcp_server", id.to_string())),
        json!({ "kind": kind, "config": config }),
    )
    .await;
    // a folder of its own, apart from the running server's
    let dir = state.config.mcp_dir.join(format!("check-{}", new_id()));
    let answer = match mcp::Conn::start(&def, &dir, state.config.mcp_stdio).await {
        Ok(conn) => json!({ "ok": true, "tools": conn.tools }),
        Err(e) => json!({ "ok": false, "error": e }),
    };
    // (the program has ended with its connection)
    let _ = tokio::fs::remove_dir_all(&dir).await;
    Ok(Json(answer))
}
