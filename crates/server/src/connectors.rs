//! Knowledge sources for retrieval-augmented answers. A connector is an internal system the
//! model can search while answering — WikiCraft today. People connect their own (read-only)
//! token, so every search runs with their permissions there: Agora never sees pages the asking
//! person couldn't open themselves.

use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::app::AppState;
use crate::llm::{SearchHit, ToolDef};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Connector {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub description: Option<String>,
    pub enabled: bool,
}

/// A connector the person has connected, ready to search.
#[derive(Debug, Clone)]
pub struct Linked {
    pub connector: Connector,
    pub token: String,
    /// the tool name the model calls
    pub tool: String,
}

/// The person's connected, enabled sources (tokens decrypted).
pub async fn linked(state: &AppState, user: Uuid) -> anyhow::Result<Vec<Linked>> {
    let rows: Vec<(Uuid, String, String, String, Option<String>, bool, Vec<u8>)> = sqlx::query_as(
        "SELECT c.id, c.name, c.kind, c.base_url, c.description, c.enabled, t.token_enc
           FROM connectors c JOIN connector_tokens t ON t.connector_id = c.id
          WHERE t.user_id = $1 AND c.enabled ORDER BY c.name",
    )
    .bind(user)
    .fetch_all(&state.db)
    .await?;
    let mut out: Vec<Linked> = Vec::new();
    for (id, name, kind, base_url, description, enabled, enc) in rows {
        let Ok(token) = state.secrets.decrypt(&enc) else {
            tracing::warn!(connector = %name, "a connector token can't be decrypted (AGORA_SECRET_KEY changed?)");
            continue;
        };
        let mut tool = format!("search_{}", slug(&name));
        if out.iter().any(|l| l.tool == tool) {
            tool = format!("{tool}_{}", &id.simple().to_string()[..6]);
        }
        out.push(Linked { connector: Connector { id, name, kind, base_url, description, enabled }, token, tool });
    }
    Ok(out)
}

pub fn slug(s: &str) -> String {
    let s: String = s.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let s = s.split('_').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("_");
    let s: String = s.chars().take(40).collect();
    if s.is_empty() { "knowledge".into() } else { s }
}

pub fn tool_def(l: &Linked) -> ToolDef {
    let about = l.connector.description.as_deref().filter(|d| !d.trim().is_empty()).unwrap_or("the organization's internal documentation");
    ToolDef {
        name: l.tool.clone(),
        description: format!(
            "Search {} (a WikiCraft wiki: {about}). Returns the passages that best answer the query, with links. Use it for questions about the organization, its projects, processes and decisions. Ask focused questions; search again with other words if the first results miss.",
            l.connector.name
        ),
        schema: json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "What you're looking for, as a question or keywords" },
                "space": { "type": "string", "description": "Optional: limit to one space key" }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    }
}

/// One passage found in a source.
#[derive(Debug, Clone)]
pub struct Passage {
    pub title: String,
    pub url: String,
    pub location: String,
    pub text: String,
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(30)).build().expect("http client")
}

/// Searches a WikiCraft wiki with the person's token (`POST /api/retrieve`: hybrid keyword +
/// semantic ranking, filtered by their permissions).
pub async fn search(l: &Linked, query: &str, space: Option<&str>) -> Result<Vec<Passage>, String> {
    let base = l.connector.base_url.trim_end_matches('/');
    let mut body = json!({ "query": query, "limit": 8, "perPage": 3, "maxChars": 2500 });
    if let Some(s) = space.filter(|s| !s.trim().is_empty()) {
        body["spaces"] = json!([s.trim()]);
    }
    let resp = http()
        .post(format!("{base}/api/retrieve"))
        .bearer_auth(&l.token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("{} can't be reached: {e}", l.connector.name))?;
    let status = resp.status().as_u16();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    match status {
        200 => {}
        401 => return Err(format!("the connection to {} isn't valid any more: reconnect it under Settings", l.connector.name)),
        _ => return Err(format!("{} answered HTTP {status}: {}", l.connector.name, v["message"].as_str().unwrap_or(""))),
    }
    Ok(v["passages"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let mut loc: Vec<String> = Vec::new();
            if let Some(s) = p["spaceName"].as_str() {
                loc.push(s.to_string());
            }
            loc.extend(p["path"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)));
            loc.extend(p["headings"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)));
            let url = p["attachment"]["url"].as_str().or(p["url"].as_str()).unwrap_or_default();
            let url = if url.starts_with('/') { format!("{base}{url}") } else { url.to_string() };
            Passage {
                title: p["attachment"]["filename"].as_str().or(p["title"].as_str()).unwrap_or("Untitled").to_string(),
                url,
                location: loc.join(" › "),
                text: p["text"].as_str().unwrap_or_default().to_string(),
            }
        })
        .collect())
}

pub fn for_model(name: &str, passages: &[Passage]) -> String {
    if passages.is_empty() {
        return format!("Nothing found in {name}. Try other words, or tell the user the wiki doesn't seem to cover it.");
    }
    passages
        .iter()
        .enumerate()
        .map(|(i, p)| format!("[{}] {}\n{}\n{}\n\n{}", i + 1, p.title, p.location, p.url, p.text.trim()))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

pub fn hits(passages: &[Passage]) -> Vec<SearchHit> {
    passages.iter().map(|p| SearchHit { title: p.title.clone(), url: p.url.clone(), snippet: p.location.clone() }).collect()
}

/// Checks a token: `GET /api/me` must answer with a user.
pub async fn check_token(base_url: &str, token: &str) -> Result<String, String> {
    let resp = http()
        .get(format!("{}/api/me", base_url.trim_end_matches('/')))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("the wiki can't be reached: {e}"))?;
    if resp.status().as_u16() == 401 {
        return Err("the wiki doesn't accept this token".into());
    }
    let v: Value = resp.json().await.map_err(|_| "the address doesn't look like a WikiCraft wiki".to_string())?;
    let user = &v["user"];
    if user.is_null() {
        return Err("the wiki doesn't accept this token".into());
    }
    Ok(user["name"].as_str().or(user["email"].as_str()).unwrap_or("?").to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn tool_names() {
        assert_eq!(super::slug("Team Wiki (Prod)"), "team_wiki_prod");
        assert_eq!(super::slug("!!!"), "knowledge");
    }
}
