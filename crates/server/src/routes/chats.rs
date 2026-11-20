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
use crate::auth::{Client, CurrentUser, MaybeDevice, User};
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
    pub project_id: Option<Uuid>,
    pub settings: Value,
    pub leaf_id: Option<Uuid>,
    pub pinned: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

const CHAT_COLUMNS: &str = "id, user_id, title, model_id, system_prompt, project_id, settings, leaf_id, pinned, created_at, updated_at, deleted_at";

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
    /// only the chats of this project
    project: Option<Uuid>,
}

pub async fn list(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(q): Query<ListQuery>) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(200).clamp(1, 500);
    let pattern =
        q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")));
    let rows: Vec<(Uuid, String, bool, Option<Uuid>, chrono::DateTime<chrono::Utc>, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "SELECT c.id, c.title, c.pinned, c.model_id, c.updated_at, c.project_id, p.name FROM chats c LEFT JOIN projects p ON p.id = c.project_id
          WHERE c.user_id = $1 AND c.deleted_at IS NULL AND ($5::uuid IS NULL OR c.project_id = $5)
            AND ($2::text IS NULL OR c.title ILIKE $2 OR EXISTS (SELECT 1 FROM messages m WHERE m.chat_id = c.id AND m.content ILIKE $2))
            AND ($4::timestamptz IS NULL OR c.updated_at < $4)
          ORDER BY c.pinned DESC, c.updated_at DESC LIMIT $3",
    )
    .bind(user.id)
    .bind(pattern)
    .bind(limit)
    .bind(q.before)
    .bind(q.project)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(
        rows.iter()
            .map(|(id, title, pinned, model, at, project, project_name)| json!({
                "id": id, "title": title, "pinned": pinned, "modelId": model, "updatedAt": at, "projectId": project, "project": project_name,
            }))
            .collect::<Vec<_>>()
    )))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CreateChat {
    model_id: Option<Uuid>,
    system_prompt: Option<String>,
    settings: Option<Value>,
    project_id: Option<Uuid>,
}

pub async fn create(State(state): State<AppState>, CurrentUser(user): CurrentUser, client: Client, Json(req): Json<CreateChat>) -> ApiResult<Json<ChatRow>> {
    if let Some(m) = req.model_id {
        crate::catalog::for_user(&state, &user, m).await?;
    }
    if let Some(p) = req.project_id {
        crate::routes::projects::can_use(&state, &user, p).await?;
    }
    let id = new_id();
    sqlx::query("INSERT INTO chats (id, user_id, model_id, system_prompt, settings, project_id) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(id)
        .bind(user.id)
        .bind(req.model_id)
        .bind(req.system_prompt.filter(|s| !s.trim().is_empty()))
        .bind(req.settings.filter(Value::is_object).unwrap_or_else(|| json!({})))
        .bind(req.project_id)
        .execute(&state.db)
        .await?;
    audit::log(
        &state,
        Some(&user),
        Some(&client),
        "chat.created",
        Some(("chat", id.to_string())),
        json!({ "modelId": req.model_id, "projectId": req.project_id }),
    )
    .await;
    Ok(Json(load_chat(&state, id).await?))
}

pub async fn get(State(state): State<AppState>, CurrentUser(user): CurrentUser, Path(id): Path<Uuid>) -> ApiResult<Json<Value>> {
    let chat = own_chat(&state, &user, id).await?;
    let project: Option<String> = match chat.project_id {
        Some(p) => sqlx::query_scalar("SELECT name FROM projects WHERE id = $1").bind(p).fetch_optional(&state.db).await?,
        None => None,
    };
    let messages = load_messages(&state, id).await?;
    let live: Vec<Uuid> = messages.iter().filter(|m| m.status == "streaming" && state.live.contains_key(&m.id)).map(|m| m.id).collect();
    Ok(Json(json!({ "chat": chat, "project": project, "messages": messages, "live": live })))
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
    /// moves the chat into a project, or out of its project (`null`)
    #[serde(default, deserialize_with = "present")]
    project_id: Option<Option<Uuid>>,
}

/// Tells a field that is `null` from one that is missing.
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
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
    if let Some(project) = req.project_id {
        if let Some(p) = project {
            crate::routes::projects::can_use(&state, &user, p).await?;
        }
        sqlx::query("UPDATE chats SET project_id = $2 WHERE id = $1").bind(id).bind(project).execute(&state.db).await?;
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
    /// from the desktop app: which tools on the person's computer this answer may use
    desktop: Option<crate::desktop::Wanted>,
    /// images and PDFs sent with the message
    #[serde(default)]
    attachments: Vec<AttachmentIn>,
    /// `compact`: summarize the conversation so far instead of answering
    command: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentIn {
    name: String,
    media_type: String,
    /// base64
    data: String,
}

const MAX_ATTACHMENTS: usize = 8;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

/// What models can be given as a file.
pub fn media_allowed(media_type: &str) -> bool {
    matches!(media_type, "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "application/pdf")
}

/// Checks and decodes what was attached to a message.
fn attachments(model: &crate::catalog::ModelRow, list: &[AttachmentIn]) -> ApiResult<Vec<(String, String, Vec<u8>)>> {
    use base64::Engine;
    if list.is_empty() {
        return Ok(vec![]);
    }
    if list.len() > MAX_ATTACHMENTS {
        return Err(ApiError::bad(format!("at most {MAX_ATTACHMENTS} files per message")));
    }
    if !model.cap("vision") {
        return Err(ApiError::bad("this model can't read images or PDFs; choose another model or remove the files"));
    }
    let mut out = Vec::new();
    let mut total = 0;
    for a in list {
        if !media_allowed(&a.media_type) {
            return Err(ApiError::bad(format!("{}: only images (PNG, JPEG, GIF, WebP) and PDFs can be attached", a.name)));
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(a.data.trim()).map_err(|_| ApiError::bad(format!("{}: not valid base64", a.name)))?;
        let limit = if a.media_type == "application/pdf" { MAX_ATTACHMENT_BYTES } else { MAX_IMAGE_BYTES };
        total += bytes.len();
        if bytes.is_empty() || bytes.len() > limit || total > MAX_ATTACHMENT_BYTES {
            return Err(ApiError::bad(format!("{}: too large (images up to 5 MB, 20 MB per message)", a.name)));
        }
        let name: String = a.name.trim().chars().filter(|c| !c.is_control()).take(200).collect();
        out.push((if name.is_empty() { "file".to_string() } else { name }, a.media_type.clone(), bytes));
    }
    Ok(out)
}

/// `GET /api/attachments/{id}`: a file of one of the person's own messages.
pub async fn attachment(State(state): State<AppState>, CurrentUser(user): CurrentUser, Path(id): Path<Uuid>) -> ApiResult<Response> {
    let row: Option<(String, String, Vec<u8>)> = sqlx::query_as(
        "SELECT a.name, a.media_type, a.data FROM attachments a JOIN messages m ON m.id = a.message_id JOIN chats c ON c.id = m.chat_id
          WHERE a.id = $1 AND c.user_id = $2 AND c.deleted_at IS NULL",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;
    let (_, media_type, data) = row.ok_or(ApiError::NotFound("file"))?;
    // only the types we accepted, and never rendered as a page of our origin
    let headers = [
        (axum::http::header::CONTENT_TYPE, media_type),
        (axum::http::header::CONTENT_DISPOSITION, "inline".to_string()),
        (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        (axum::http::header::CONTENT_SECURITY_POLICY, "sandbox; default-src 'none'".to_string()),
        (axum::http::header::CACHE_CONTROL, "private, max-age=86400".to_string()),
    ];
    Ok((headers, data).into_response())
}

/// `POST /api/chats/{id}/messages`: sends a message and streams the answer (SSE).
pub async fn send(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    MaybeDevice(device): MaybeDevice,
    client: Client,
    Path(id): Path<Uuid>,
    Json(req): Json<SendRequest>,
) -> ApiResult<Response> {
    let chat = own_chat(&state, &user, id).await?;
    let content = req.content.trim_end().to_string();
    if content.trim().is_empty() && req.attachments.is_empty() {
        return Err(ApiError::bad("the message is empty"));
    }
    let command = req.command.as_deref().map(str::trim).filter(|c| !c.is_empty());
    if command.is_some_and(|c| c != "compact") {
        return Err(ApiError::bad("unknown command"));
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
    let files = attachments(&model, &req.attachments)?;
    let user_msg = new_id();
    let assistant_msg = new_id();
    let mut tx = state.db.begin().await?;
    let files: Vec<(Uuid, String, String, Vec<u8>)> = files.into_iter().map(|(name, kind, data)| (new_id(), name, kind, data)).collect();
    let parts: Vec<Value> =
        files.iter().map(|(fid, name, kind, data)| json!({ "type": "attachment", "id": fid, "name": name, "mediaType": kind, "size": data.len() })).collect();
    sqlx::query("INSERT INTO messages (id, chat_id, parent_id, role, content, parts) VALUES ($1, $2, $3, 'user', $4, $5)")
        .bind(user_msg)
        .bind(id)
        .bind(req.parent_id)
        .bind(&content)
        .bind(Value::Array(parts))
        .execute(&mut *tx)
        .await?;
    for (fid, name, kind, data) in &files {
        sqlx::query("INSERT INTO attachments (id, message_id, name, media_type, size_bytes, data) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(fid)
            .bind(user_msg)
            .bind(name)
            .bind(kind)
            .bind(data.len() as i32)
            .bind(data)
            .execute(&mut *tx)
            .await?;
    }
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
                "chars": content.chars().count(), "content": full.then_some(&content),
                "attachments": files.iter().map(|f| json!({ "name": f.1, "mediaType": f.2, "size": f.3.len() })).collect::<Vec<_>>(), "command": command }),
    )
    .await;
    let title_after = req.parent_id.is_none() && chat.title == "New chat" && command.is_none();
    let desktop = desktop_session(&state, &user, device, req.desktop.as_ref()).await;
    let command = command.map(String::from);
    stream_answer(&state, user, client, settings, &chat, model, user_msg, assistant_msg, web, knowledge, req.effort, title_after, desktop, command).await
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
    desktop: Option<crate::desktop::Wanted>,
}

/// The tools of the person's desktop app for an answer: only for requests the app itself made.
async fn desktop_session(state: &AppState, user: &User, device: Option<Uuid>, wanted: Option<&crate::desktop::Wanted>) -> Option<crate::desktop::Session> {
    crate::desktop::session(state, user, device?, wanted?).await
}

/// `POST /api/chats/{id}/regenerate`: another answer to a message (a new branch).
pub async fn regenerate(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    MaybeDevice(device): MaybeDevice,
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
    let desktop = desktop_session(&state, &user, device, req.desktop.as_ref()).await;
    stream_answer(&state, user, client, settings, &chat, model, m.id, assistant_msg, web, knowledge, req.effort, false, desktop, None).await
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
    desktop: Option<crate::desktop::Session>,
    command: Option<String>,
) -> ApiResult<Response> {
    let effort =
        effort.filter(|e| matches!(e.as_str(), "low" | "medium" | "high" | "xhigh" | "max")).or_else(|| chat.settings["effort"].as_str().map(String::from));
    let first = json!({
        "type": "start",
        "userMessage": load_message(state, user_msg).await?,
        "assistantMessage": load_message(state, assistant_msg).await?,
    });
    let project = crate::routes::projects::context(state, &user, chat.project_id).await;
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
        project,
        effort,
        title_after,
        desktop,
        command,
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
