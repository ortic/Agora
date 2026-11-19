//! Agora Desktop: signing a computer in, the app's connection, and what administrators decide
//! about it (features, local connectors, devices, the log of tool calls).

use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::app::{AppState, new_id};
use crate::audit;
use crate::auth::{AdminUser, AuditorUser, Client, CurrentUser, MaybeDevice, random_token, token_hash};
use crate::desktop::{self, Device, Reply};
use crate::error::{ApiError, ApiResult};
use crate::llm::ToolDef;

// ---- signing a device in -------------------------------------------------------------------

const LOGIN_MINUTES: i32 = 10;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartLogin {
    device_name: String,
    platform: Option<String>,
    app_version: Option<String>,
}

fn short(s: &str, n: usize) -> String {
    s.trim().chars().filter(|c| !c.is_control()).take(n).collect()
}

/// A code people can read out and type: 8 letters and digits without the look-alikes.
fn user_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut buf = [0u8; 8];
    rand::fill(&mut buf);
    let s: String = buf.iter().map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char).collect();
    format!("{}-{}", &s[..4], &s[4..])
}

/// `POST /api/desktop/login/start` (no sign-in): the app asks for a code to show.
pub async fn login_start(State(state): State<AppState>, client: Client, Json(req): Json<StartLogin>) -> ApiResult<Json<Value>> {
    sqlx::query("DELETE FROM device_logins WHERE expires_at < now()").execute(&state.db).await?;
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM device_logins WHERE ip IS NOT DISTINCT FROM $1").bind(&client.ip).fetch_one(&state.db).await?;
    if pending >= 20 {
        return Err(ApiError::Limit("too many sign-ins waiting from this address: try again in a few minutes".into()));
    }
    let name = short(&req.device_name, 80);
    if name.is_empty() {
        return Err(ApiError::bad("the device needs a name"));
    }
    let poll = random_token();
    let code = user_code();
    sqlx::query(
        "INSERT INTO device_logins (poll_hash, user_code, device_name, platform, app_version, ip, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now() + make_interval(mins => $7))",
    )
    .bind(token_hash(&poll))
    .bind(&code)
    .bind(&name)
    .bind(req.platform.as_deref().map(|p| short(p, 40)))
    .bind(req.app_version.as_deref().map(|p| short(p, 40)))
    .bind(&client.ip)
    .bind(LOGIN_MINUTES)
    .execute(&state.db)
    .await?;
    let base = state.config.base_url.trim_end_matches('/');
    Ok(Json(json!({
        "pollToken": poll, "userCode": code, "verifyUrl": format!("{base}/desktop/link?code={code}"),
        "expiresIn": LOGIN_MINUTES * 60, "interval": 2,
    })))
}

fn normal_code(code: &str) -> String {
    let c: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect();
    if c.len() == 8 { format!("{}-{}", &c[..4], &c[4..]) } else { c }
}

/// `GET /api/desktop/login/{code}`: what the person is about to confirm.
pub async fn login_info(State(state): State<AppState>, CurrentUser(user): CurrentUser, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    let row: Option<(String, Option<String>, Option<String>, Option<String>, String, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("SELECT device_name, platform, app_version, ip, status, created_at FROM device_logins WHERE user_code = $1 AND expires_at > now()")
            .bind(normal_code(&code))
            .fetch_optional(&state.db)
            .await?;
    let Some((name, platform, version, ip, status, at)) = row else {
        return Err(ApiError::NotFound("sign-in code"));
    };
    Ok(Json(json!({
        "deviceName": name, "platform": platform, "appVersion": version, "ip": ip, "status": status, "createdAt": at,
        "allowed": desktop::may_use_desktop(&state, user.id).await,
    })))
}

#[derive(Deserialize)]
pub struct Decision {
    approve: bool,
}

/// `POST /api/desktop/login/{code}`: the signed-in person confirms (or refuses) the device.
pub async fn login_decide(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    client: Client,
    Path(code): Path<String>,
    Json(d): Json<Decision>,
) -> ApiResult<StatusCode> {
    if d.approve && !desktop::may_use_desktop(&state, user.id).await {
        return Err(ApiError::Denied("Agora Desktop isn't enabled for your account. Ask an administrator.".into()));
    }
    let name: Option<String> = sqlx::query_scalar(
        "UPDATE device_logins SET status = $2, user_id = $3 WHERE user_code = $1 AND status = 'pending' AND expires_at > now() RETURNING device_name",
    )
    .bind(normal_code(&code))
    .bind(if d.approve { "approved" } else { "denied" })
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;
    let Some(name) = name else {
        return Err(ApiError::NotFound("sign-in code"));
    };
    audit::log(&state, Some(&user), Some(&client), if d.approve { "device.approved" } else { "device.denied" }, None, json!({ "deviceName": name })).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Poll {
    poll_token: String,
}

/// `POST /api/desktop/login/poll`: the app waits for the confirmation; the token is handed out
/// once.
pub async fn login_poll(State(state): State<AppState>, client: Client, Json(req): Json<Poll>) -> ApiResult<Json<Value>> {
    let hash = token_hash(&req.poll_token);
    let row: Option<(String, Option<Uuid>, String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT status, user_id, device_name, platform, app_version FROM device_logins WHERE poll_hash = $1 AND expires_at > now()")
            .bind(&hash)
            .fetch_optional(&state.db)
            .await?;
    let Some((status, user_id, name, platform, version)) = row else {
        return Ok(Json(json!({ "status": "expired" })));
    };
    match (status.as_str(), user_id) {
        ("approved", Some(user_id)) => {
            let taken =
                sqlx::query("DELETE FROM device_logins WHERE poll_hash = $1 AND status = 'approved'").bind(&hash).execute(&state.db).await?.rows_affected();
            if taken == 0 {
                return Ok(Json(json!({ "status": "expired" })));
            }
            let user = crate::routes::auth::load_user(&state, user_id).await?;
            let token = format!("{}{}", desktop::DEVICE_TOKEN_PREFIX, random_token());
            let id = new_id();
            sqlx::query("INSERT INTO devices (id, user_id, name, platform, app_version, token_hash, last_seen_at) VALUES ($1, $2, $3, $4, $5, $6, now())")
                .bind(id)
                .bind(user_id)
                .bind(&name)
                .bind(&platform)
                .bind(&version)
                .bind(token_hash(&token))
                .execute(&state.db)
                .await?;
            audit::log(
                &state,
                Some(&user),
                Some(&client),
                "device.signed_in",
                Some(("device", id.to_string())),
                json!({ "name": name, "platform": platform, "appVersion": version }),
            )
            .await;
            Ok(Json(json!({ "status": "approved", "token": token, "deviceId": id, "user": user })))
        }
        ("denied", _) => {
            sqlx::query("DELETE FROM device_logins WHERE poll_hash = $1").bind(&hash).execute(&state.db).await?;
            Ok(Json(json!({ "status": "denied" })))
        }
        _ => Ok(Json(json!({ "status": "pending" }))),
    }
}

// ---- the person's devices ------------------------------------------------------------------

type DeviceRow = (
    Uuid,
    Uuid,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    chrono::DateTime<chrono::Utc>,
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
);

async fn device_rows(state: &AppState, user: Option<Uuid>) -> ApiResult<Vec<Value>> {
    let rows: Vec<DeviceRow> = sqlx::query_as(
        "SELECT d.id, d.user_id, u.email, d.name, d.platform, d.app_version, d.created_at, d.last_seen_at, d.revoked_at
           FROM devices d JOIN users u ON u.id = d.user_id
          WHERE ($1::uuid IS NULL OR d.user_id = $1) AND (d.revoked_at IS NULL OR d.revoked_at > now() - interval '30 days')
          ORDER BY d.revoked_at IS NOT NULL, d.last_seen_at DESC NULLS LAST",
    )
    .bind(user)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, user_id, email, name, platform, version, created, seen, revoked)| {
            json!({
                "id": id, "userId": user_id, "user": email, "name": name, "platform": platform, "appVersion": version, "createdAt": created,
                "lastSeenAt": seen, "revokedAt": revoked, "connected": state.devices.contains_key(&id),
            })
        })
        .collect())
}

pub async fn my_devices(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<Value>> {
    Ok(Json(json!(device_rows(&state, Some(user.id)).await?)))
}

async fn revoke(state: &AppState, id: Uuid, owner: Option<Uuid>) -> ApiResult<String> {
    let name: Option<String> =
        sqlx::query_scalar("UPDATE devices SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL AND ($2::uuid IS NULL OR user_id = $2) RETURNING name")
            .bind(id)
            .bind(owner)
            .fetch_optional(&state.db)
            .await?;
    let name = name.ok_or(ApiError::NotFound("device"))?;
    if let Some((_, dev)) = state.devices.remove(&id) {
        let _ = dev.tx.send(json!({ "type": "bye", "reason": "This device was signed out." }));
        dev.close();
    }
    Ok(name)
}

pub async fn revoke_my_device(State(state): State<AppState>, CurrentUser(user): CurrentUser, client: Client, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let name = revoke(&state, id, Some(user.id)).await?;
    audit::log(&state, Some(&user), Some(&client), "device.revoked", Some(("device", id.to_string())), json!({ "name": name })).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/desktop/policy`: what the signed-in person's app may do.
pub async fn my_policy(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<desktop::Policy>> {
    Ok(Json(desktop::policy(&state, user.id).await?))
}

// ---- the app's connection ------------------------------------------------------------------

/// `GET /api/desktop/ws`: the app connects with its device token and stays connected; tool
/// calls go down this socket, results come back.
pub async fn ws(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    MaybeDevice(device): MaybeDevice,
    upgrade: WebSocketUpgrade,
) -> ApiResult<Response> {
    let Some(device) = device else {
        return Err(ApiError::Denied("only a signed-in Agora Desktop app can connect here".into()));
    };
    let policy = desktop::policy(&state, user.id).await?;
    if !policy.desktop {
        return Err(ApiError::Denied("Agora Desktop isn't enabled for your account".into()));
    }
    Ok(upgrade.max_message_size(8 * 1024 * 1024).on_upgrade(move |socket| serve(state, user.id, device, policy, socket)))
}

async fn serve(state: AppState, user_id: Uuid, device_id: Uuid, policy: desktop::Policy, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let dev = Device::new(user_id, tx.clone());
    if let Some(old) = state.devices.insert(device_id, dev.clone()) {
        let _ = old.tx.send(json!({ "type": "bye", "reason": "The app connected again from elsewhere." }));
        old.close();
    }
    tracing::info!(device = %device_id, "desktop app connected");
    let _ = tx.send(json!({ "type": "policy", "policy": policy }));
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let bye = msg["type"] == "bye";
            if sink.send(Message::Text(msg.to_string().into())).await.is_err() || bye {
                break;
            }
        }
        let _ = sink.close().await;
    });
    let mut idle = tokio::time::interval(std::time::Duration::from_secs(25));
    loop {
        let msg = tokio::select! {
            m = stream.next() => m,
            _ = idle.tick() => {
                if tx.send(json!({ "type": "ping" })).is_err() { break; }
                continue;
            }
        };
        let text = match msg {
            Some(Ok(Message::Text(t))) => t,
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
            Some(Ok(_)) => continue,
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        match v["type"].as_str().unwrap_or_default() {
            "tools" => {
                let mut map = std::collections::HashMap::new();
                for c in v["connectors"].as_array().into_iter().flatten() {
                    let Some(id) = c["id"].as_str().and_then(|s| s.parse::<Uuid>().ok()) else { continue };
                    let tools: Vec<ToolDef> = c["tools"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|t| {
                            Some(ToolDef {
                                name: t["name"].as_str().filter(|n| !n.is_empty())?.chars().take(120).collect(),
                                description: t["description"].as_str().unwrap_or_default().chars().take(4000).collect(),
                                schema: t["inputSchema"].clone(),
                            })
                        })
                        .take(200)
                        .collect();
                    map.insert(id, tools);
                }
                *dev.tools.lock().expect("device tools") = map;
            }
            "progress" => {
                if let (Some(id), Some(text)) = (v["id"].as_str(), v["text"].as_str()) {
                    dev.reply(id, Reply::Progress(text.to_string()));
                }
            }
            "result" => {
                if let Some(id) = v["id"].as_str() {
                    dev.reply(
                        id,
                        Reply::Result {
                            content: v["content"].as_str().unwrap_or_default().to_string(),
                            is_error: v["isError"].as_bool().unwrap_or(false),
                            status: v["status"].as_str().unwrap_or("ok").to_string(),
                            media: returned_media(&v["media"]),
                        },
                    );
                }
            }
            _ => {}
        }
    }
    state.devices.remove_if(&device_id, |_, d| d.conn_id == dev.conn_id);
    dev.close();
    writer.abort();
    let _ = sqlx::query("UPDATE devices SET last_seen_at = now() WHERE id = $1").bind(device_id).execute(&state.db).await;
    tracing::info!(device = %device_id, "desktop app disconnected");
}

// ---- administration ------------------------------------------------------------------------

/// `GET /api/admin/desktop`: features, local connectors and devices.
pub async fn admin_get(State(state): State<AppState>, AdminUser(_): AdminUser) -> ApiResult<Json<Value>> {
    let features: Vec<(String, String, Value, Vec<Uuid>)> = sqlx::query_as(
        "SELECT f.key, f.access, f.config, coalesce((SELECT array_agg(group_id) FROM desktop_feature_groups g WHERE g.key = f.key), '{}') FROM desktop_features f",
    )
    .fetch_all(&state.db)
    .await?;
    let mut fs = serde_json::Map::new();
    for (key, access, config, groups) in features {
        // stored settings over the defaults, so the form always has every field
        let mut full = match key.as_str() {
            "code" => serde_json::to_value(desktop::CodeConfig::default())?,
            "browser" => serde_json::to_value(desktop::BrowserConfig::default())?,
            _ => serde_json::to_value(desktop::GeneralConfig::default())?,
        };
        if let (Some(f), Some(c)) = (full.as_object_mut(), config.as_object()) {
            f.extend(c.clone());
        }
        fs.insert(key, json!({ "access": access, "config": full, "groupIds": groups }));
    }
    let connectors: Vec<(Uuid, String, String, Option<String>, Value, Vec<String>, String, bool, String, Vec<Uuid>)> = sqlx::query_as(
        "SELECT c.id, c.name, c.kind, c.description, c.config, c.tools, c.approval, c.enabled, c.access,
                coalesce((SELECT array_agg(group_id) FROM local_connector_groups g WHERE g.connector_id = c.id), '{}')
           FROM local_connectors c ORDER BY c.name",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "features": fs,
        "connectors": connectors.into_iter().map(|(id, name, kind, description, config, tools, approval, enabled, access, groups)| json!({
            "id": id, "name": name, "kind": kind, "description": description, "config": config, "tools": tools, "approval": approval,
            "enabled": enabled, "access": access, "groupIds": groups,
        })).collect::<Vec<_>>(),
        "devices": device_rows(&state, None).await?,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureBody {
    access: String,
    config: Option<Value>,
    group_ids: Option<Vec<Uuid>>,
}

/// `PUT /api/admin/desktop/features/{key}`
pub async fn admin_put_feature(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    client: Client,
    Path(key): Path<String>,
    Json(req): Json<FeatureBody>,
) -> ApiResult<StatusCode> {
    if !matches!(req.access.as_str(), "nobody" | "all" | "groups") {
        return Err(ApiError::bad("access is nobody, all or groups"));
    }
    // normalised through the typed settings: unknown fields go, wrong types are refused
    let config = match (key.as_str(), req.config.clone().filter(Value::is_object)) {
        (_, None) => None,
        ("desktop", Some(c)) => {
            let mut c: desktop::GeneralConfig = serde_json::from_value(c)?;
            for cmd in &mut c.commands {
                cmd.name = cmd.name.trim().trim_start_matches('/').to_ascii_lowercase();
                if cmd.name.is_empty() || !cmd.name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
                    return Err(ApiError::bad("a command's name is letters, digits, - and _"));
                }
                if cmd.prompt.trim().is_empty() {
                    return Err(ApiError::bad(format!("/{} has no prompt", cmd.name)));
                }
            }
            Some(serde_json::to_value(c)?)
        }
        ("code", Some(c)) => {
            let mut c: desktop::CodeConfig = serde_json::from_value(c)?;
            c.images = c.images.iter().map(|i| i.trim().to_string()).filter(|i| !i.is_empty()).collect();
            c.default_image = c.default_image.trim().to_string();
            if c.default_image.is_empty() {
                return Err(ApiError::bad("give a default image"));
            }
            if !matches!(c.approval.as_str(), "ask" | "auto") {
                return Err(ApiError::bad("approval is ask or auto"));
            }
            Some(serde_json::to_value(c)?)
        }
        ("browser", Some(c)) => {
            let mut c: desktop::BrowserConfig = serde_json::from_value(c)?;
            let clean = |l: &[String]| l.iter().map(|d| d.trim().to_ascii_lowercase()).filter(|d| !d.is_empty()).collect::<Vec<_>>();
            c.allowed_domains = clean(&c.allowed_domains);
            c.blocked_domains = clean(&c.blocked_domains);
            if !matches!(c.approval.as_str(), "ask" | "auto") {
                return Err(ApiError::bad("approval is ask or auto"));
            }
            Some(serde_json::to_value(c)?)
        }
        _ => return Err(ApiError::NotFound("feature")),
    };
    let mut tx = state.db.begin().await?;
    let n = sqlx::query("UPDATE desktop_features SET access = $2, config = coalesce($3, config), updated_at = now() WHERE key = $1")
        .bind(&key)
        .bind(&req.access)
        .bind(&config)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("feature"));
    }
    if let Some(groups) = &req.group_ids {
        sqlx::query("DELETE FROM desktop_feature_groups WHERE key = $1").bind(&key).execute(&mut *tx).await?;
        for g in groups {
            sqlx::query("INSERT INTO desktop_feature_groups (key, group_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(&key)
                .bind(g)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    audit::log(
        &state,
        Some(&admin),
        Some(&client),
        "desktop.feature_updated",
        Some(("desktop_feature", key.clone())),
        json!({ "access": req.access, "config": config, "groups": req.group_ids }),
    )
    .await;
    desktop::push_policies(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorBody {
    name: Option<String>,
    kind: Option<String>,
    description: Option<String>,
    config: Option<Value>,
    tools: Option<Vec<String>>,
    approval: Option<String>,
    enabled: Option<bool>,
    access: Option<String>,
    group_ids: Option<Vec<Uuid>>,
}

/// Checks a connector's settings and keeps only what its kind uses.
fn connector_config(kind: &str, c: &Value) -> ApiResult<Value> {
    let strings = |v: &Value| -> serde_json::Map<String, Value> {
        v.as_object().map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), json!(s)))).collect()).unwrap_or_default()
    };
    match kind {
        "mcp_stdio" => {
            let command = c["command"].as_str().unwrap_or_default().trim();
            if command.is_empty() {
                return Err(ApiError::bad("give the command that starts the server"));
            }
            let args: Vec<&str> = c["args"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            Ok(json!({ "command": command, "args": args, "env": strings(&c["env"]) }))
        }
        "mcp_http" => {
            let u = c["url"].as_str().unwrap_or_default().trim();
            let parsed = url::Url::parse(u).map_err(|_| ApiError::bad("give the server's URL"))?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(ApiError::bad("the URL starts with http:// or https://"));
            }
            Ok(json!({ "url": u, "headers": strings(&c["headers"]) }))
        }
        _ => Err(ApiError::bad("kind is mcp_stdio or mcp_http")),
    }
}

async fn set_connector_groups(state: &AppState, id: Uuid, groups: &[Uuid]) -> ApiResult<()> {
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM local_connector_groups WHERE connector_id = $1").bind(id).execute(&mut *tx).await?;
    for g in groups {
        sqlx::query("INSERT INTO local_connector_groups (connector_id, group_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(id)
            .bind(g)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn check_choice(what: &str, value: Option<&str>, allowed: &[&str]) -> ApiResult<()> {
    match value {
        Some(v) if !allowed.contains(&v) => Err(ApiError::bad(format!("{what} is one of: {}", allowed.join(", ")))),
        _ => Ok(()),
    }
}

/// `POST /api/admin/desktop/connectors`
pub async fn admin_create_connector(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    client: Client,
    Json(req): Json<ConnectorBody>,
) -> ApiResult<Json<Value>> {
    let name = short(req.name.as_deref().unwrap_or_default(), 80);
    if name.is_empty() {
        return Err(ApiError::bad("give the connector a name"));
    }
    let kind = req.kind.as_deref().unwrap_or("mcp_stdio");
    let config = connector_config(kind, req.config.as_ref().unwrap_or(&Value::Null))?;
    check_choice("approval", req.approval.as_deref(), &["ask", "auto"])?;
    check_choice("access", req.access.as_deref(), &["all", "groups"])?;
    let id = new_id();
    sqlx::query(
        "INSERT INTO local_connectors (id, name, kind, description, config, tools, approval, enabled, access) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(&name)
    .bind(kind)
    .bind(req.description.as_deref().map(str::trim).filter(|d| !d.is_empty()))
    .bind(&config)
    .bind(req.tools.clone().unwrap_or_default())
    .bind(req.approval.as_deref().unwrap_or("ask"))
    .bind(req.enabled.unwrap_or(true))
    .bind(req.access.as_deref().unwrap_or("groups"))
    .execute(&state.db)
    .await?;
    if let Some(g) = &req.group_ids {
        set_connector_groups(&state, id, g).await?;
    }
    audit::log(
        &state,
        Some(&admin),
        Some(&client),
        "desktop.connector_created",
        Some(("local_connector", id.to_string())),
        json!({ "name": name, "kind": kind, "config": config, "access": req.access, "groups": req.group_ids }),
    )
    .await;
    desktop::push_policies(&state).await;
    Ok(Json(json!({ "id": id })))
}

/// `PATCH /api/admin/desktop/connectors/{id}`
pub async fn admin_update_connector(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    client: Client,
    Path(id): Path<Uuid>,
    Json(req): Json<ConnectorBody>,
) -> ApiResult<StatusCode> {
    let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM local_connectors WHERE id = $1").bind(id).fetch_optional(&state.db).await?;
    let stored = kind.ok_or(ApiError::NotFound("connector"))?;
    let kind = req.kind.clone().unwrap_or(stored);
    check_choice("approval", req.approval.as_deref(), &["ask", "auto"])?;
    check_choice("access", req.access.as_deref(), &["all", "groups"])?;
    let config = match &req.config {
        Some(c) => Some(connector_config(&kind, c)?),
        None if req.kind.is_some() => return Err(ApiError::bad("a connector of another kind needs its settings")),
        None => None,
    };
    let name = req.name.as_deref().map(|n| short(n, 80)).filter(|n| !n.is_empty());
    sqlx::query(
        "UPDATE local_connectors SET name = coalesce($2, name), kind = $3, config = coalesce($4, config), tools = coalesce($5, tools),
                approval = coalesce($6, approval), enabled = coalesce($7, enabled), access = coalesce($8, access),
                description = CASE WHEN $9 THEN $10 ELSE description END, updated_at = now()
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
    .execute(&state.db)
    .await?;
    if let Some(g) = &req.group_ids {
        set_connector_groups(&state, id, g).await?;
    }
    audit::log(
        &state,
        Some(&admin),
        Some(&client),
        "desktop.connector_updated",
        Some(("local_connector", id.to_string())),
        json!({ "name": name, "kind": kind, "config": config, "tools": req.tools, "approval": req.approval, "enabled": req.enabled, "access": req.access, "groups": req.group_ids }),
    )
    .await;
    desktop::push_policies(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/admin/desktop/connectors/{id}`
pub async fn admin_delete_connector(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let name: Option<String> = sqlx::query_scalar("DELETE FROM local_connectors WHERE id = $1 RETURNING name").bind(id).fetch_optional(&state.db).await?;
    let name = name.ok_or(ApiError::NotFound("connector"))?;
    audit::log(&state, Some(&admin), Some(&client), "desktop.connector_deleted", Some(("local_connector", id.to_string())), json!({ "name": name })).await;
    desktop::push_policies(&state).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/admin/desktop/devices/{id}`: signs a device out.
pub async fn admin_revoke_device(State(state): State<AppState>, AdminUser(admin): AdminUser, client: Client, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let name = revoke(&state, id, None).await?;
    audit::log(&state, Some(&admin), Some(&client), "device.revoked", Some(("device", id.to_string())), json!({ "name": name, "byAdmin": true })).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolQuery {
    user_id: Option<Uuid>,
    feature: Option<String>,
    q: Option<String>,
    before: Option<chrono::DateTime<chrono::Utc>>,
    limit: Option<i64>,
}

/// `GET /api/admin/desktop/tools`: what models did on people's computers.
pub async fn admin_tool_events(State(state): State<AppState>, AuditorUser(_): AuditorUser, Query(q): Query<ToolQuery>) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let pattern = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| format!("%{s}%"));
    type Row = (
        Uuid,
        chrono::DateTime<chrono::Utc>,
        Option<Uuid>,
        Option<String>,
        Option<Uuid>,
        Option<String>,
        String,
        Option<String>,
        String,
        Value,
        String,
        Option<String>,
        i32,
        Option<i32>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT t.id, t.created_at, t.user_id, u.email, t.chat_id, d.name, t.feature, t.connector, t.tool, t.input, t.status, t.error, t.result_chars, t.duration_ms
           FROM tool_events t LEFT JOIN users u ON u.id = t.user_id LEFT JOIN devices d ON d.id = t.device_id
          WHERE ($1::uuid IS NULL OR t.user_id = $1) AND ($2::text IS NULL OR t.feature = $2)
            AND ($3::text IS NULL OR t.tool ILIKE $3 OR t.input::text ILIKE $3 OR t.connector ILIKE $3)
            AND ($4::timestamptz IS NULL OR t.created_at < $4)
          ORDER BY t.created_at DESC LIMIT $5",
    )
    .bind(q.user_id)
    .bind(q.feature.as_deref().filter(|k| !k.is_empty()))
    .bind(pattern)
    .bind(q.before)
    .bind(limit)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({
        "events": rows.iter().map(|r| json!({
            "id": r.0, "at": r.1, "userId": r.2, "user": r.3, "chatId": r.4, "device": r.5, "feature": r.6, "connector": r.7, "tool": r.8,
            "input": r.9, "status": r.10, "error": r.11, "resultChars": r.12, "durationMs": r.13,
        })).collect::<Vec<_>>(),
        "more": rows.len() as i64 == limit,
    })))
}

/// The files a tool result carries: a few images or PDFs of bounded size.
fn returned_media(v: &Value) -> Vec<crate::llm::Media> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let media_type = m["mediaType"].as_str()?;
            let data = m["data"].as_str()?;
            (crate::routes::chats::media_allowed(media_type) && data.len() <= 9_000_000).then(|| crate::llm::Media {
                media_type: media_type.to_string(),
                data: data.to_string(),
                name: m["name"].as_str().unwrap_or("file").chars().take(200).collect(),
            })
        })
        .take(4)
        .collect()
}
