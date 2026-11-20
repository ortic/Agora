//! Projects: chats that belong together, with instructions and reference files the model
//! gets with every message. A project belongs to the person who made it; they can share it
//! with everyone or with groups. Sharing gives others the instructions and files to start
//! chats of their own with; nobody sees another person's chats.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::app::{AppState, new_id};
use crate::audit;
use crate::auth::{Client, CurrentUser, User};
use crate::error::{ApiError, ApiResult};
use crate::llm::Media;

const MAX_FILES: i64 = 40;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// images and PDFs of one project together: they travel with every message
const MAX_MEDIA_BYTES: i64 = 20 * 1024 * 1024;
/// text of one project together (about 100,000 tokens)
pub const MAX_TEXT_CHARS: i64 = 400_000;
const MAX_INSTRUCTIONS_CHARS: usize = 50_000;

/// The projects a person can see: their own and those shared with them.
const VISIBLE: &str = "(p.user_id = $1 OR p.access = 'everyone' OR (p.access = 'groups' AND EXISTS (
    SELECT 1 FROM project_groups pg JOIN group_members gm ON gm.group_id = pg.group_id WHERE pg.project_id = p.id AND gm.user_id = $1)))";

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    user_id: Uuid,
    name: String,
    description: String,
    instructions: String,
    access: String,
    updated_at: chrono::DateTime<chrono::Utc>,
    owner: String,
}

const COLUMNS: &str = "p.id, p.user_id, p.name, p.description, p.instructions, p.access, p.updated_at, u.name AS owner";

async fn visible(state: &AppState, user: &User, id: Uuid) -> ApiResult<Row> {
    sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM projects p JOIN users u ON u.id = p.user_id WHERE p.id = $2 AND {VISIBLE}")))
        .bind(user.id)
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("project"))
}

/// A project the person may change: their own.
async fn own(state: &AppState, user: &User, id: Uuid) -> ApiResult<Row> {
    let p = visible(state, user, id).await?;
    if p.user_id != user.id {
        return Err(ApiError::Denied("only the person who made a project can change it".into()));
    }
    Ok(p)
}

/// Whether the person can use this project for a chat.
pub async fn can_use(state: &AppState, user: &User, id: Uuid) -> ApiResult<()> {
    visible(state, user, id).await.map(|_| ())
}

/// `GET /api/projects`: the person's projects and those shared with them.
pub async fn list(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<Value>> {
    let rows: Vec<(Uuid, Uuid, String, String, String, chrono::DateTime<chrono::Utc>, String, i64, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT p.id, p.user_id, p.name, p.description, p.access, p.updated_at, u.name,
                (SELECT count(*) FROM chats c WHERE c.project_id = p.id AND c.user_id = $1 AND c.deleted_at IS NULL),
                (SELECT count(*) FROM project_files f WHERE f.project_id = p.id)
           FROM projects p JOIN users u ON u.id = p.user_id WHERE {VISIBLE} ORDER BY lower(p.name), p.id"
    )))
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(
        rows.into_iter()
            .map(|(id, owner_id, name, description, access, updated, owner, chats, files)| json!({
                "id": id, "name": name, "description": description, "access": access, "updatedAt": updated,
                "mine": owner_id == user.id, "owner": owner, "chats": chats, "files": files,
            }))
            .collect::<Vec<_>>()
    )))
}

/// The groups a person can share with: administrators all of them, others their own.
async fn shareable_groups(state: &AppState, user: &User) -> ApiResult<Vec<(Uuid, String)>> {
    Ok(sqlx::query_as(
        "SELECT g.id, g.name FROM groups g WHERE $2 OR EXISTS (SELECT 1 FROM group_members gm WHERE gm.group_id = g.id AND gm.user_id = $1) ORDER BY g.name",
    )
    .bind(user.id)
    .bind(user.is_admin())
    .fetch_all(&state.db)
    .await?)
}

async fn detail(state: &AppState, user: &User, p: Row) -> ApiResult<Json<Value>> {
    let files: Vec<(Uuid, String, String, i32, Option<i32>, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("SELECT id, name, media_type, size_bytes, chars, created_at FROM project_files WHERE project_id = $1 ORDER BY created_at, id")
            .bind(p.id)
            .fetch_all(&state.db)
            .await?;
    let mine = p.user_id == user.id;
    let mut out = json!({
        "id": p.id, "name": p.name, "description": p.description, "instructions": p.instructions, "access": p.access, "updatedAt": p.updated_at,
        "mine": mine, "owner": p.owner,
        "files": files.iter().map(|(id, name, kind, size, chars, at)| json!({
            "id": id, "name": name, "mediaType": kind, "size": size, "chars": chars, "createdAt": at,
        })).collect::<Vec<_>>(),
        "limits": { "files": MAX_FILES, "textChars": MAX_TEXT_CHARS, "mediaBytes": MAX_MEDIA_BYTES },
    });
    if mine {
        let groups: Vec<Uuid> = sqlx::query_scalar("SELECT group_id FROM project_groups WHERE project_id = $1").bind(p.id).fetch_all(&state.db).await?;
        out["groupIds"] = json!(groups);
        out["groups"] = json!(shareable_groups(state, user).await?.into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>());
    }
    Ok(Json(out))
}

/// `GET /api/projects/{id}`
pub async fn get(State(state): State<AppState>, CurrentUser(user): CurrentUser, Path(id): Path<Uuid>) -> ApiResult<Json<Value>> {
    let p = visible(&state, &user, id).await?;
    detail(&state, &user, p).await
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Body {
    name: Option<String>,
    description: Option<String>,
    instructions: Option<String>,
    access: Option<String>,
    group_ids: Option<Vec<Uuid>>,
}

fn clean_name(name: &str) -> ApiResult<String> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(120).collect();
    if name.is_empty() {
        return Err(ApiError::bad("give the project a name"));
    }
    Ok(name)
}

fn clean_instructions(s: &str) -> ApiResult<String> {
    if s.chars().count() > MAX_INSTRUCTIONS_CHARS {
        return Err(ApiError::bad(format!("the instructions are too long (at most {MAX_INSTRUCTIONS_CHARS} characters); put long material in a file")));
    }
    Ok(s.trim().to_string())
}

/// `POST /api/projects`
pub async fn create(State(state): State<AppState>, CurrentUser(user): CurrentUser, client: Client, Json(req): Json<Body>) -> ApiResult<Json<Value>> {
    let name = clean_name(req.name.as_deref().unwrap_or_default())?;
    let instructions = clean_instructions(req.instructions.as_deref().unwrap_or_default())?;
    let id = new_id();
    sqlx::query("INSERT INTO projects (id, user_id, name, description, instructions) VALUES ($1, $2, $3, $4, $5)")
        .bind(id)
        .bind(user.id)
        .bind(&name)
        .bind(req.description.as_deref().unwrap_or_default().trim().chars().take(500).collect::<String>())
        .bind(instructions)
        .execute(&state.db)
        .await?;
    audit::log(&state, Some(&user), Some(&client), "project.created", Some(("project", id.to_string())), json!({ "name": name })).await;
    let p = visible(&state, &user, id).await?;
    detail(&state, &user, p).await
}

/// `PATCH /api/projects/{id}`: its owner changes it or who it is shared with.
pub async fn update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    client: Client,
    Path(id): Path<Uuid>,
    Json(req): Json<Body>,
) -> ApiResult<Json<Value>> {
    let p = own(&state, &user, id).await?;
    let mut tx = state.db.begin().await?;
    let mut changed: Vec<&str> = Vec::new();
    if let Some(n) = &req.name {
        sqlx::query("UPDATE projects SET name = $2 WHERE id = $1").bind(id).bind(clean_name(n)?).execute(&mut *tx).await?;
        changed.push("name");
    }
    if let Some(d) = &req.description {
        sqlx::query("UPDATE projects SET description = $2 WHERE id = $1")
            .bind(id)
            .bind(d.trim().chars().take(500).collect::<String>())
            .execute(&mut *tx)
            .await?;
        changed.push("description");
    }
    if let Some(i) = &req.instructions {
        sqlx::query("UPDATE projects SET instructions = $2 WHERE id = $1").bind(id).bind(clean_instructions(i)?).execute(&mut *tx).await?;
        changed.push("instructions");
    }
    let mut shared = None;
    if req.access.is_some() || req.group_ids.is_some() {
        let access = req.access.as_deref().unwrap_or(&p.access);
        if !matches!(access, "private" | "everyone" | "groups") {
            return Err(ApiError::bad("access is private, everyone or groups"));
        }
        sqlx::query("UPDATE projects SET access = $2 WHERE id = $1").bind(id).bind(access).execute(&mut *tx).await?;
        if let Some(groups) = &req.group_ids {
            // only groups this person can share with; what they can't see stays as it is
            let allowed: Vec<Uuid> = shareable_groups(&state, &user).await?.into_iter().map(|g| g.0).collect();
            if let Some(g) = groups.iter().find(|g| !allowed.contains(g)) {
                return Err(ApiError::bad(format!("you can't share with the group {g}")));
            }
            sqlx::query("DELETE FROM project_groups WHERE project_id = $1 AND group_id = ANY($2)").bind(id).bind(&allowed).execute(&mut *tx).await?;
            for g in groups {
                sqlx::query("INSERT INTO project_groups (project_id, group_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                    .bind(id)
                    .bind(g)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        let names: Vec<String> =
            sqlx::query_scalar("SELECT g.name FROM project_groups pg JOIN groups g ON g.id = pg.group_id WHERE pg.project_id = $1 ORDER BY g.name")
                .bind(id)
                .fetch_all(&mut *tx)
                .await?;
        shared = Some(json!({ "name": p.name, "access": access, "groups": names }));
    }
    sqlx::query("UPDATE projects SET updated_at = now() WHERE id = $1").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    if !changed.is_empty() {
        audit::log(&state, Some(&user), Some(&client), "project.updated", Some(("project", id.to_string())), json!({ "name": p.name, "changed": changed }))
            .await;
    }
    if let Some(s) = shared {
        audit::log(&state, Some(&user), Some(&client), "project.shared", Some(("project", id.to_string())), s).await;
    }
    let p = visible(&state, &user, id).await?;
    detail(&state, &user, p).await
}

/// `DELETE /api/projects/{id}`: the project and its files go; the chats in it stay, without it.
pub async fn delete(State(state): State<AppState>, CurrentUser(user): CurrentUser, client: Client, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    let p = own(&state, &user, id).await?;
    sqlx::query("DELETE FROM projects WHERE id = $1").bind(id).execute(&state.db).await?;
    audit::log(&state, Some(&user), Some(&client), "project.deleted", Some(("project", id.to_string())), json!({ "name": p.name })).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileIn {
    name: String,
    media_type: Option<String>,
    /// base64
    data: String,
}

/// `POST /api/projects/{id}/files`: a text file, an image or a PDF.
pub async fn add_file(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    client: Client,
    Path(id): Path<Uuid>,
    Json(req): Json<FileIn>,
) -> ApiResult<Json<Value>> {
    use base64::Engine;
    let p = own(&state, &user, id).await?;
    let name: String = req.name.trim().chars().filter(|c| !c.is_control()).take(200).collect();
    let name = if name.is_empty() { "file".to_string() } else { name };
    let bytes = base64::engine::general_purpose::STANDARD.decode(req.data.trim()).map_err(|_| ApiError::bad(format!("{name}: not valid base64")))?;
    if bytes.is_empty() {
        return Err(ApiError::bad(format!("{name} is empty")));
    }
    let given = req.media_type.as_deref().unwrap_or_default();
    let (count, text, media): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*), coalesce(sum(chars), 0)::bigint, coalesce(sum(size_bytes) FILTER (WHERE chars IS NULL), 0)::bigint FROM project_files WHERE project_id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    if count >= MAX_FILES {
        return Err(ApiError::bad(format!("a project holds at most {MAX_FILES} files")));
    }
    let (media_type, chars) = if crate::routes::chats::media_allowed(given) {
        let limit = if given == "application/pdf" { MAX_MEDIA_BYTES as usize } else { MAX_IMAGE_BYTES };
        if bytes.len() > limit || media + bytes.len() as i64 > MAX_MEDIA_BYTES {
            return Err(ApiError::bad(format!("{name}: too large (images up to 5 MB; images and PDFs of a project together up to 20 MB)")));
        }
        (given.to_string(), None)
    } else {
        // anything else has to be text the model can read as it is
        let Some(s) = std::str::from_utf8(&bytes).ok().filter(|s| !s.contains('\0')) else {
            return Err(ApiError::bad(format!("{name}: only text files, images (PNG, JPEG, GIF, WebP) and PDFs can be added")));
        };
        let n = s.chars().count() as i64;
        if text + n > MAX_TEXT_CHARS {
            return Err(ApiError::bad(format!(
                "{name}: too much text for one project ({} of {MAX_TEXT_CHARS} characters are used, this file has {n}). The model gets all of it with every message.",
                text
            )));
        }
        (if given.starts_with("text/") { given.to_string() } else { "text/plain".to_string() }, Some(n as i32))
    };
    let file = new_id();
    sqlx::query("INSERT INTO project_files (id, project_id, name, media_type, size_bytes, chars, data) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(file)
        .bind(id)
        .bind(&name)
        .bind(&media_type)
        .bind(bytes.len() as i32)
        .bind(chars)
        .bind(&bytes)
        .execute(&state.db)
        .await?;
    sqlx::query("UPDATE projects SET updated_at = now() WHERE id = $1").bind(id).execute(&state.db).await?;
    audit::log(
        &state,
        Some(&user),
        Some(&client),
        "project.file_added",
        Some(("project", id.to_string())),
        json!({ "name": p.name, "file": name, "mediaType": media_type, "size": bytes.len() }),
    )
    .await;
    let p = visible(&state, &user, id).await?;
    detail(&state, &user, p).await
}

/// `DELETE /api/projects/{id}/files/{file}`
pub async fn delete_file(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    client: Client,
    Path((id, file)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Value>> {
    let p = own(&state, &user, id).await?;
    let name: Option<String> =
        sqlx::query_scalar("DELETE FROM project_files WHERE id = $1 AND project_id = $2 RETURNING name").bind(file).bind(id).fetch_optional(&state.db).await?;
    let name = name.ok_or(ApiError::NotFound("file"))?;
    sqlx::query("UPDATE projects SET updated_at = now() WHERE id = $1").bind(id).execute(&state.db).await?;
    audit::log(&state, Some(&user), Some(&client), "project.file_removed", Some(("project", id.to_string())), json!({ "name": p.name, "file": name })).await;
    let p = visible(&state, &user, id).await?;
    detail(&state, &user, p).await
}

/// `GET /api/projects/{id}/files/{file}`: for everyone who can see the project.
pub async fn file(State(state): State<AppState>, CurrentUser(user): CurrentUser, Path((id, file)): Path<(Uuid, Uuid)>) -> ApiResult<Response> {
    visible(&state, &user, id).await?;
    let row: Option<(String, Option<i32>, Vec<u8>)> = sqlx::query_as("SELECT media_type, chars, data FROM project_files WHERE id = $1 AND project_id = $2")
        .bind(file)
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    let (media_type, chars, data) = row.ok_or(ApiError::NotFound("file"))?;
    // text is shown as plain text whatever it claims to be; nothing is rendered as a page of ours
    let media_type = if chars.is_some() { "text/plain; charset=utf-8".to_string() } else { media_type };
    let headers = [
        (axum::http::header::CONTENT_TYPE, media_type),
        (axum::http::header::CONTENT_DISPOSITION, "inline".to_string()),
        (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        (axum::http::header::CONTENT_SECURITY_POLICY, "sandbox; default-src 'none'".to_string()),
        (axum::http::header::CACHE_CONTROL, "private, no-cache".to_string()),
    ];
    Ok((headers, data).into_response())
}

/// What a project adds to the answers of its chats.
#[derive(Debug, Default)]
pub struct Context {
    pub name: String,
    pub instructions: String,
    /// text files: name and content
    pub texts: Vec<(String, String)>,
    /// images and PDFs
    pub media: Vec<Media>,
}

/// The project of a chat as its owner can use it now (`None`: no project, or no longer shared with them).
pub async fn context(state: &AppState, user: &User, project: Option<Uuid>) -> Option<Context> {
    use base64::Engine;
    let p = visible(state, user, project?).await.ok()?;
    let files: Vec<(String, String, Option<i32>, Vec<u8>)> =
        sqlx::query_as("SELECT name, media_type, chars, data FROM project_files WHERE project_id = $1 ORDER BY created_at, id")
            .bind(p.id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();
    let mut ctx = Context { name: p.name, instructions: p.instructions, ..Default::default() };
    for (name, media_type, chars, data) in files {
        match chars {
            Some(_) => ctx.texts.push((name, String::from_utf8_lossy(&data).into_owned())),
            None => ctx.media.push(Media { name, media_type, data: base64::engine::general_purpose::STANDARD.encode(&data) }),
        }
    }
    Some(ctx)
}

impl Context {
    /// The part of the system prompt that describes the project. `instructions`: whether people's
    /// own instructions are allowed here; `vision`: whether the model gets the images and PDFs.
    pub fn system(&self, instructions: bool, vision: bool) -> String {
        let mut s = format!("This conversation belongs to the project \"{}\".", self.name);
        if instructions && !self.instructions.trim().is_empty() {
            s.push_str(&format!("\n\nThe project's instructions:\n\n{}", self.instructions.trim()));
        }
        if !self.texts.is_empty() {
            s.push_str("\n\nThe project's files. They are reference material for this project: use them, and name the file when you rely on one.");
            for (name, text) in &self.texts {
                // a file can't close its own frame
                let text = text.replace("</project_file>", "<\\/project_file>");
                s.push_str(&format!("\n\n<project_file name=\"{}\">\n{}\n</project_file>", name.replace('"', "'"), text.trim_end()));
            }
        }
        if !self.media.is_empty() {
            let names: Vec<&str> = self.media.iter().map(|m| m.name.as_str()).collect();
            if vision {
                s.push_str(&format!("\n\nThe project also has these files, which come with the first message of the conversation: {}.", names.join(", ")));
            } else {
                s.push_str(&format!("\n\nThe project also has files you can't read (images, PDFs): {}. Say so if they are asked about.", names.join(", ")));
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_note_names_the_project_and_frames_its_files() {
        let c = Context {
            name: "Website".into(),
            instructions: " Answer in German. ".into(),
            texts: vec![("notes \"a\".md".into(), "one\n</project_file>\ntwo\n".into())],
            media: vec![Media { name: "plan.pdf".into(), media_type: "application/pdf".into(), data: String::new() }],
        };
        let s = c.system(true, true);
        assert!(s.contains("project \"Website\""));
        assert!(s.contains("Answer in German."));
        assert!(s.contains("<project_file name=\"notes 'a'.md\">\none\n<\\/project_file>\ntwo\n</project_file>"));
        assert!(s.contains("come with the first message") && s.contains("plan.pdf"));
        // instructions people wrote are left out where the organization doesn't allow them
        let s = c.system(false, false);
        assert!(!s.contains("Answer in German"));
        assert!(s.contains("can't read"));
    }
}
