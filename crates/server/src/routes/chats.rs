//! Chats and messages of the signed-in person.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::app::{AppState, new_id};
use crate::audit;
use crate::auth::{Client, CurrentUser, User};
use crate::chat::{self, Job};
use crate::error::{ApiError, ApiResult};

pub const MAX_MESSAGE_CHARS: usize = 400_000;

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ChatRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: String,
    pub model_id: Option<Uuid>,
    pub system_prompt: Option<String>,
    pub settings: Value,
    pub leaf_id: Option<Uuid>,
    pub pinned: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

const CHAT_COLUMNS: &str = "id, user_id, title, model_id, system_prompt, settings, leaf_id, pinned, created_at, updated_at, deleted_at";

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MessageRow {
    pub id: Uuid,
    pub chat_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub role: String,
    pub content: String,
    pub parts: Value,
    pub model_id: Option<Uuid>,
    pub model_name: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    pub duration_ms: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

const MESSAGE_COLUMNS: &str =
    "id, chat_id, parent_id, role, content, parts, model_id, model_name, status, error, input_tokens, output_tokens, cost_usd, duration_ms, created_at";

pub async fn load_message(state: &AppState, id: Uuid) -> ApiResult<MessageRow> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = $1")))
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("message"))
}

pub async fn load_messages(state: &AppState, chat: Uuid) -> ApiResult<Vec<MessageRow>> {
    Ok(sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE chat_id = $1 ORDER BY created_at, id")))
        .bind(chat)
        .fetch_all(&state.db)
        .await?)
}

pub async fn load_chat(state: &AppState, id: Uuid) -> ApiResult<ChatRow> {
    sqlx::query_as::<_, ChatRow>(sqlx::AssertSqlSafe(format!("SELECT {CHAT_COLUMNS} FROM chats WHERE id = $1")))
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("chat"))
}

/// The person's own chat (deleted chats are gone for them).
async fn own_chat(state: &AppState, user: &User, id: Uuid) -> ApiResult<ChatRow> {
    let c = load_chat(state, id).await?;
    if c.user_id != user.id || c.deleted_at.is_some() {
        return Err(ApiError::NotFound("chat"));
    }
    Ok(c)
}

#[derive(Deserialize)]
pub struct ListQuery {
    q: Option<String>,
    limit: Option<i64>,
    before: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn list(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(q): Query<ListQuery>) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(200).clamp(1, 500);
    let pattern =
        q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")));
    let rows: Vec<(Uuid, String, bool, Option<Uuid>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT c.id, c.title, c.pinned, c.model_id, c.updated_at FROM chats c
          WHERE c.user_id = $1 AND c.deleted_at IS NULL
            AND ($2::text IS NULL OR c.title ILIKE $2 OR EXISTS (SELECT 1 FROM messages m WHERE m.chat_id = c.id AND m.content ILIKE $2))
            AND ($4::timestamptz IS NULL OR c.updated_at < $4)
          ORDER BY c.pinned DESC, c.updated_at DESC LIMIT $3",
    )
    .bind(user.id)
    .bind(pattern)
    .bind(limit)
    .bind(q.before)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(
        rows.iter()
            .map(|(id, title, pinned, model, at)| json!({ "id": id, "title": title, "pinned": pinned, "modelId": model, "updatedAt": at }))
            .collect::<Vec<_>>()
    )))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CreateChat {
    model_id: Option<Uuid>,
    system_prompt: Option<String>,
    settings: Option<Value>,
}

pub async fn create(State(state): State<AppState>, CurrentUser(user): CurrentUser, client: Client, Json(req): Json<CreateChat>) -> ApiResult<Json<ChatRow>> {
    if let Some(m) = req.model_id {
        crate::catalog::for_user(&state, &user, m).await?;
    }
    let id = new_id();
    sqlx::query("INSERT INTO chats (id, user_id, model_id, system_prompt, settings) VALUES ($1, $2, $3, $4, $5)")
        .bind(id)
        .bind(user.id)
        .bind(req.model_id)
        .bind(req.system_prompt.filter(|s| !s.trim().is_empty()))
        .bind(req.settings.filter(Value::is_object).unwrap_or_else(|| json!({})))
        .execute(&state.db)
        .await?;
    audit::log(&state, Some(&user), Some(&client), "chat.created", Some(("chat", id.to_string())), json!({ "modelId": req.model_id })).await;
    Ok(Json(load_chat(&state, id).await?))
}

pub async fn get(State(state): State<AppState>, CurrentUser(user): CurrentUser, Path(id): Path<Uuid>) -> ApiResult<Json<Value>> {
    let chat = own_chat(&state, &user, id).await?;
    let messages = load_messages(&state, id).await?;
    let live: Vec<Uuid> = messages.iter().filter(|m| m.status == "streaming" && state.live.contains_key(&m.id)).map(|m| m.id).collect();
    Ok(Json(json!({ "chat": chat, "messages": messages, "live": live })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChat {
    title: Option<String>,
    pinned: Option<bool>,
    model_id: Option<Uuid>,
    system_prompt: Option<String>,
    settings: Option<Value>,
    leaf_id: Option<Uuid>,
}

pub async fn update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateChat>,
) -> ApiResult<Json<ChatRow>> {
    let chat = own_chat(&state, &user, id).await?;
    if let Some(t) = &req.title {
        let t = t.trim();
        if t.is_empty() {
            return Err(ApiError::bad("the title can't be empty"));
        }
        sqlx::query("UPDATE chats SET title = $2 WHERE id = $1").bind(id).bind(t.chars().take(200).collect::<String>()).execute(&state.db).await?;
    }
    if let Some(p) = req.pinned {
        sqlx::query("UPDATE chats SET pinned = $2 WHERE id = $1").bind(id).bind(p).execute(&state.db).await?;
    }
    if let Some(m) = req.model_id {
        crate::catalog::for_user(&state, &user, m).await?;
        sqlx::query("UPDATE chats SET model_id = $2 WHERE id = $1").bind(id).bind(m).execute(&state.db).await?;
    }
    if let Some(s) = &req.system_prompt {
        sqlx::query("UPDATE chats SET system_prompt = $2 WHERE id = $1").bind(id).bind(Some(s.trim()).filter(|s| !s.is_empty())).execute(&state.db).await?;
    }
    if let Some(Value::Object(s)) = &req.settings {
        let mut merged = chat.settings.as_object().cloned().unwrap_or_default();
        merged.extend(s.clone());
        sqlx::query("UPDATE chats SET settings = $2 WHERE id = $1").bind(id).bind(Value::Object(merged)).execute(&state.db).await?;
    }
    if let Some(leaf) = req.leaf_id {
        let ok: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM messages WHERE id = $1 AND chat_id = $2)").bind(leaf).bind(id).fetch_one(&state.db).await?;
        if !ok {
            return Err(ApiError::bad("that message isn't in this chat"));
        }
        sqlx::query("UPDATE chats SET leaf_id = $2 WHERE id = $1").bind(id).bind(leaf).execute(&state.db).await?;
    }
    Ok(Json(load_chat(&state, id).await?))
}

/// Deleting hides the chat from its owner; it stays available to auditors until the retention
/// period ends (`chatRetentionDays`).
pub async fn delete(State(state): State<AppState>, CurrentUser(user): CurrentUser, client: Client, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let chat = own_chat(&state, &user, id).await?;
    sqlx::query("UPDATE chats SET deleted_at = now() WHERE id = $1").bind(id).execute(&state.db).await?;
    audit::log(&state, Some(&user), Some(&client), "chat.deleted", Some(("chat", id.to_string())), json!({ "title": chat.title })).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendRequest {
    content: String,
    /// the message this one follows (`null`: the start of the chat); editing a message sends
    /// its replacement after the same parent, which starts a new branch
    parent_id: Option<Uuid>,
    model_id: Option<Uuid>,
    web_search: Option<bool>,
    knowledge: Option<bool>,
    effort: Option<String>,
}

/// `POST /api/chats/{id}/messages`: sends a message and streams the answer (SSE).
pub async fn send(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    client: Client,
    Path(id): Path<Uuid>,
    Json(req): Json<SendRequest>,
) -> ApiResult<Response> {
    let chat = own_chat(&state, &user, id).await?;
    let content = req.content.trim_end().to_string();
    if content.trim().is_empty() {
        return Err(ApiError::bad("the message is empty"));
    }
    if content.chars().count() > MAX_MESSAGE_CHARS {
        return Err(ApiError::bad(format!("the message is too long (at most {MAX_MESSAGE_CHARS} characters)")));
    }
    if let Some(p) = req.parent_id {
        let m = load_message(&state, p).await?;
        if m.chat_id != id {
            return Err(ApiError::bad("the parent message isn't in this chat"));
        }
        if m.role != "assistant" {
            return Err(ApiError::bad("a message follows an answer"));
        }
        if m.status == "streaming" {
            return Err(ApiError::Conflict("wait for the answer to finish".into()));
        }
    }
    let settings = crate::settings::load(&state).await?;
    let model_id = req.model_id.or(chat.model_id).or(settings.default_model_id).ok_or_else(|| ApiError::bad("choose a model"))?;
    let model = crate::catalog::for_user(&state, &user, model_id).await?;
    crate::usage::check(&state, &user, &settings).await?;
    let user_msg = new_id();
    let assistant_msg = new_id();
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO messages (id, chat_id, parent_id, role, content) VALUES ($1, $2, $3, 'user', $4)")
        .bind(user_msg)
        .bind(id)
        .bind(req.parent_id)
        .bind(&content)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO messages (id, chat_id, parent_id, role, model_id, model_name, status, created_at) VALUES ($1, $2, $3, 'assistant', $4, $5, 'streaming', now() + interval '1 millisecond')")
        .bind(assistant_msg)
        .bind(id)
        .bind(user_msg)
        .bind(model.id)
        .bind(&model.model)
        .execute(&mut *tx)
        .await?;
    let web = req.web_search.unwrap_or(false);
    let mut chat_settings = chat.settings.as_object().cloned().unwrap_or_default();
    chat_settings.insert("webSearch".into(), json!(web));
    let knowledge = req.knowledge.unwrap_or(false);
    chat_settings.insert("knowledge".into(), json!(knowledge));
    sqlx::query("UPDATE chats SET leaf_id = $2, model_id = $3, settings = $4, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(assistant_msg)
        .bind(model.id)
        .bind(Value::Object(chat_settings))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let full = settings.audit_content == "full";
    audit::log(
        &state,
        Some(&user),
        Some(&client),
        "chat.prompt",
        Some(("chat", id.to_string())),
        json!({ "messageId": user_msg, "model": model.model, "modelId": model.id, "webSearch": web, "edit": req.parent_id.is_none() && chat.leaf_id.is_some(),
                "chars": content.chars().count(), "content": full.then_some(&content) }),
    )
    .await;
    let title_after = req.parent_id.is_none() && chat.title == "New chat";
    stream_answer(&state, user, client, settings, &chat, model, user_msg, assistant_msg, web, knowledge, req.effort, title_after).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegenerateRequest {
    /// the user message to answer again
    message_id: Uuid,
    model_id: Option<Uuid>,
    web_search: Option<bool>,
    knowledge: Option<bool>,
    effort: Option<String>,
}

/// `POST /api/chats/{id}/regenerate`: another answer to a message (a new branch).
pub async fn regenerate(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    client: Client,
    Path(id): Path<Uuid>,
    Json(req): Json<RegenerateRequest>,
) -> ApiResult<Response> {
    let chat = own_chat(&state, &user, id).await?;
    let m = load_message(&state, req.message_id).await?;
    if m.chat_id != id || m.role != "user" {
        return Err(ApiError::bad("choose a message of yours in this chat"));
    }
    let settings = crate::settings::load(&state).await?;
    let model_id = req.model_id.or(chat.model_id).or(settings.default_model_id).ok_or_else(|| ApiError::bad("choose a model"))?;
    let model = crate::catalog::for_user(&state, &user, model_id).await?;
    crate::usage::check(&state, &user, &settings).await?;
    let assistant_msg = new_id();
    sqlx::query("INSERT INTO messages (id, chat_id, parent_id, role, model_id, model_name, status) VALUES ($1, $2, $3, 'assistant', $4, $5, 'streaming')")
        .bind(assistant_msg)
        .bind(id)
        .bind(m.id)
        .bind(model.id)
        .bind(&model.model)
        .execute(&state.db)
        .await?;
    sqlx::query("UPDATE chats SET leaf_id = $2, model_id = $3, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(assistant_msg)
        .bind(model.id)
        .execute(&state.db)
        .await?;
    audit::log(
        &state,
        Some(&user),
        Some(&client),
        "chat.regenerate",
        Some(("chat", id.to_string())),
        json!({ "messageId": m.id, "model": model.model, "modelId": model.id }),
    )
    .await;
    let web = req.web_search.unwrap_or_else(|| chat.settings["webSearch"].as_bool().unwrap_or(false));
    let knowledge = req.knowledge.unwrap_or_else(|| chat.settings["knowledge"].as_bool().unwrap_or(false));
    stream_answer(&state, user, client, settings, &chat, model, m.id, assistant_msg, web, knowledge, req.effort, false).await
}

async fn stream_answer(
    state: &AppState,
    user: User,
    client: Client,
    settings: crate::settings::Settings,
    chat: &ChatRow,
    model: crate::catalog::ModelRow,
    user_msg: Uuid,
    assistant_msg: Uuid,
    web: bool,
    knowledge: bool,
    effort: Option<String>,
    title_after: bool,
) -> ApiResult<Response> {
    let effort =
        effort.filter(|e| matches!(e.as_str(), "low" | "medium" | "high" | "xhigh" | "max")).or_else(|| chat.settings["effort"].as_str().map(String::from));
    let first = json!({
        "type": "start",
        "userMessage": load_message(state, user_msg).await?,
        "assistantMessage": load_message(state, assistant_msg).await?,
    });
    let job = Job {
        state: state.clone(),
        user,
        client,
        settings,
        chat_id: chat.id,
        model,
        user_msg,
        assistant_msg,
        web,
        knowledge,
        chat_system: chat.system_prompt.clone(),
        effort,
        title_after,
    };
    let (hub, _snap, seq, rx) = chat::start(job);
    let stream = chat::sse_stream(hub, vec![first], seq, rx);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()).into_response())
}

/// `GET /api/messages/{id}/stream`: follow an answer that is being written (e.g. after a reload).
pub async fn follow(State(state): State<AppState>, CurrentUser(user): CurrentUser, Path(id): Path<Uuid>) -> ApiResult<Response> {
    let m = load_message(&state, id).await?;
    own_chat(&state, &user, m.chat_id).await?;
    let hub = state.live.get(&id).map(|l| l.hub.clone());
    match hub {
        Some(hub) => {
            let (snap, seq, rx) = hub.subscribe();
            Ok(Sse::new(chat::sse_stream(hub, vec![snap], seq, rx)).keep_alive(KeepAlive::default()).into_response())
        }
        None => {
            // already finished (or on another replica): the stored state
            let events = vec![json!({ "type": "done", "message": m }), json!({ "type": "end" })];
            let stream =
                futures::stream::iter(events.into_iter().map(|e| Ok::<_, std::convert::Infallible>(axum::response::sse::Event::default().data(e.to_string()))));
            Ok(Sse::new(stream).into_response())
        }
    }
}

/// `POST /api/messages/{id}/stop`
pub async fn stop(State(state): State<AppState>, CurrentUser(user): CurrentUser, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    match state.live.get(&id) {
        Some(l) if l.user_id == user.id => {
            l.cancel.cancel();
            Ok(StatusCode::NO_CONTENT)
        }
        Some(_) => Err(ApiError::NotFound("message")),
        None => Ok(StatusCode::NO_CONTENT),
    }
}

/// Models the person may use.
pub async fn models(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<Value>> {
    let ms = crate::catalog::accessible(&state, &user).await?;
    let s = crate::settings::load(&state).await?;
    Ok(Json(json!(
        ms.iter()
            .map(|m| json!({
                "id": m.id, "name": m.display_name, "model": m.model, "description": m.description, "provider": m.provider_name,
                "kind": m.provider_kind, "contextWindow": m.context_window,
                "inputPrice": m.input_price, "outputPrice": m.output_price,
                "tools": m.cap("tools"), "vision": m.cap("vision"), "reasoning": m.cap("reasoning"),
                "effort": m.param_str("effort"),
                "webSearch": m.cap("tools") && (s.web_search.engine != "none" || (s.web_search.claude_native && m.provider_kind == "anthropic")),
            }))
            .collect::<Vec<_>>()
    )))
}
