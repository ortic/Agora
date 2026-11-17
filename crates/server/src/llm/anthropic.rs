//! Claude through the Anthropic Messages API (raw HTTP; Rust has no official SDK), streaming.

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::sse::Reader;
use super::{Block, Discovered, Endpoint, Event, LlmError, Msg, Outcome, Request, Role, SearchHit, Stop, ToolCall, Usage};

pub const API_VERSION: &str = "2026-10-21";
pub const DEFAULT_URL: &str = "https://api.anthropic.com";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// Models that take the server-side fallback when their safety classifiers decline a request.
const FALLBACK_MODELS: [&str; 5] = ["claude-fable-5-2", "claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"];

fn block_json(b: &Block) -> Option<Value> {
    Some(match b {
        Block::Text(t) if t.trim().is_empty() => return None,
        Block::Text(t) => json!({ "type": "text", "text": t }),
        Block::Image { media_type, data } => json!({ "type": "image", "source": { "type": "base64", "media_type": media_type, "data": data } }),
        Block::ToolUse { id, name, input } => json!({ "type": "tool_use", "id": id, "name": name, "input": input }),
        Block::ToolResult { id, content, is_error } => json!({ "type": "tool_result", "tool_use_id": id, "content": content, "is_error": is_error }),
    })
}

/// The `messages` array: turns in order, consecutive turns of one role merged.
pub fn messages_json(msgs: &[Msg]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut push = |role: &str, content: Vec<Value>| {
        if content.is_empty() {
            return;
        }
        if let Some(last) = out.last_mut()
            && last["role"] == role
            && let Some(arr) = last["content"].as_array_mut()
        {
            arr.extend(content);
            return;
        }
        out.push(json!({ "role": role, "content": content }));
    };
    for m in msgs {
        match m {
            Msg::Plain { role, content } => {
                let role = if *role == Role::User { "user" } else { "assistant" };
                push(role, content.iter().filter_map(block_json).collect());
            }
            Msg::AnthropicRaw { messages, .. } => {
                for raw in messages {
                    let role = raw["role"].as_str().unwrap_or("assistant").to_string();
                    let content = raw["content"].as_array().cloned().unwrap_or_default();
                    push(&role, content);
                }
            }
        }
    }
    out
}

pub fn body(req: &Request) -> Value {
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens.unwrap_or(32000),
        "stream": true,
        "messages": messages_json(&req.messages),
        // automatic prompt caching of the conversation so far (multi-turn chats re-read it cheaply)
        "cache_control": { "type": "ephemeral" },
    });
    if let Some(s) = req.system.as_deref().filter(|s| !s.trim().is_empty()) {
        body["system"] = json!(s);
    }
    if req.thinking {
        body["thinking"] = json!({ "type": "adaptive", "display": "summarized" });
    }
    if let Some(e) = req.effort.as_deref().filter(|e| !e.is_empty()) {
        body["output_config"] = json!({ "effort": e });
    }
    let mut tools: Vec<Value> =
        req.tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.schema })).collect();
    if let Some(ns) = &req.native_search {
        let mut t = json!({ "type": "web_search_20260209", "name": "web_search", "max_uses": ns.max_uses });
        if !ns.blocked_domains.is_empty() {
            t["blocked_domains"] = json!(ns.blocked_domains);
        }
        tools.push(t);
    }
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if FALLBACK_MODELS.contains(&req.model.as_str()) {
        // a declined request is re-run on Anthropic's recommended fallback model
        body["fallbacks"] = json!("default");
    }
    body
}

fn request(http: &reqwest::Client, ep: &Endpoint, path: &str) -> reqwest::RequestBuilder {
    let base = ep.base_url.trim_end_matches('/');
    let mut r = http.request(if path.starts_with("/v1/models") { reqwest::Method::GET } else { reqwest::Method::POST }, format!("{base}{path}"));
    r = r.header("anthropic-version", API_VERSION).header("x-api-key", ep.api_key.as_deref().unwrap_or_default());
    for (k, v) in &ep.headers {
        r = r.header(k, v);
    }
    r
}

/// A content block being streamed.
struct Partial {
    block: Value,
    /// accumulated `input_json_delta` for tool use blocks
    json: String,
}

pub async fn stream(
    http: &reqwest::Client,
    ep: &Endpoint,
    req: &Request,
    cancel: &CancellationToken,
    on_event: &mut (dyn FnMut(Event) + Send),
) -> Result<Outcome, LlmError> {
    let body = serde_json::to_vec(&body(req)).map_err(|e| LlmError::Http(e.to_string()))?;
    let fallback = FALLBACK_MODELS.contains(&req.model.as_str());
    let resp = super::send(
        || {
            let r = request(http, ep, "/v1/messages").header("content-type", "application/json").body(body.clone());
            if fallback { r.header("anthropic-beta", FALLBACK_BETA) } else { r }
        },
        cancel,
        ep.timeout_secs,
    )
    .await?;
    let mut reader = Reader::new(resp, ep.timeout_secs);
    let mut blocks: Vec<Value> = Vec::new();
    let mut current: Option<(u64, Partial)> = None;
    let mut usage = Usage::default();
    let mut stop = Stop::Other("incomplete".into());
    let mut model = req.model.clone();
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    while let Some(ev) = reader.next(cancel).await? {
        if ev.data.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(&ev.data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match v["type"].as_str().unwrap_or_default() {
            "message_start" => {
                let m = &v["message"];
                if let Some(s) = m["model"].as_str() {
                    model = s.to_string();
                }
                read_usage(&m["usage"], &mut usage);
            }
            "content_block_start" => {
                let idx = v["index"].as_u64().unwrap_or(0);
                let block = v["content_block"].clone();
                match block["type"].as_str().unwrap_or_default() {
                    "tool_use" => on_event(Event::ToolStart {
                        id: block["id"].as_str().unwrap_or_default().into(),
                        name: block["name"].as_str().unwrap_or_default().into(),
                    }),
                    "web_search_tool_result" => {
                        let id = block["tool_use_id"].as_str().unwrap_or_default().to_string();
                        let (results, error) = match &block["content"] {
                            Value::Array(items) => (
                                items
                                    .iter()
                                    .map(|r| SearchHit {
                                        title: r["title"].as_str().unwrap_or_default().into(),
                                        url: r["url"].as_str().unwrap_or_default().into(),
                                        snippet: r["page_age"].as_str().map(|a| format!("({a})")).unwrap_or_default(),
                                    })
                                    .collect(),
                                None,
                            ),
                            other => (vec![], Some(other["error_code"].as_str().unwrap_or("error").to_string())),
                        };
                        on_event(Event::NativeSearchResults { id, results, error });
                    }
                    _ => {}
                }
                current = Some((idx, Partial { block, json: String::new() }));
            }
            "content_block_delta" => {
                let Some((_, p)) = current.as_mut() else { continue };
                let d = &v["delta"];
                match d["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        let t = d["text"].as_str().unwrap_or_default();
                        append(&mut p.block, "text", t);
                        text.push_str(t);
                        on_event(Event::Text(t.to_string()));
                    }
                    "thinking_delta" => {
                        let t = d["thinking"].as_str().unwrap_or_default();
                        append(&mut p.block, "thinking", t);
                        on_event(Event::Reasoning(t.to_string()));
                    }
                    "signature_delta" => append(&mut p.block, "signature", d["signature"].as_str().unwrap_or_default()),
                    "input_json_delta" => p.json.push_str(d["partial_json"].as_str().unwrap_or_default()),
                    "citations_delta" => {
                        if let Some(c) = d.get("citation") {
                            match p.block["citations"].as_array_mut() {
                                Some(arr) => arr.push(c.clone()),
                                None => p.block["citations"] = json!([c]),
                            }
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let Some((_, mut p)) = current.take() else { continue };
                let ty = p.block["type"].as_str().unwrap_or_default().to_string();
                if matches!(ty.as_str(), "tool_use" | "server_tool_use") {
                    let input: Value = if p.json.trim().is_empty() { p.block["input"].clone() } else { serde_json::from_str(&p.json).unwrap_or(Value::Null) };
                    p.block["input"] = if input.is_null() { json!({}) } else { input.clone() };
                    let id = p.block["id"].as_str().unwrap_or_default().to_string();
                    let name = p.block["name"].as_str().unwrap_or_default().to_string();
                    if ty == "tool_use" {
                        tool_calls.push(ToolCall { id, name, input });
                    } else if name == "web_search" {
                        on_event(Event::NativeSearch { id, query: input["query"].as_str().unwrap_or_default().to_string() });
                    }
                }
                blocks.push(p.block);
            }
            "message_delta" => {
                read_usage(&v["usage"], &mut usage);
                if let Some(r) = v["delta"]["stop_reason"].as_str() {
                    stop = match r {
                        "end_turn" | "stop_sequence" => Stop::End,
                        "tool_use" => Stop::ToolUse,
                        "max_tokens" | "model_context_window_exceeded" => Stop::MaxTokens,
                        "refusal" => Stop::Refusal,
                        "pause_turn" => Stop::Pause,
                        other => Stop::Other(other.to_string()),
                    };
                }
            }
            "error" => {
                let msg = v["error"]["message"].as_str().unwrap_or("the model service reported an error").to_string();
                let status = if v["error"]["type"] == "overloaded_error" { 529 } else { 500 };
                return Err(LlmError::Status { status, message: msg });
            }
            _ => {}
        }
    }
    if stop == Stop::Other("incomplete".into()) {
        return Err(LlmError::Http("the model's answer broke off".into()));
    }
    Ok(Outcome { text, tool_calls, stop, usage, model, raw_content: Some(blocks) })
}

fn append(block: &mut Value, field: &str, s: &str) {
    let cur = block[field].as_str().unwrap_or_default().to_string();
    block[field] = Value::String(cur + s);
}

/// Usage fields are cumulative; later events repeat or update earlier ones.
fn read_usage(u: &Value, usage: &mut Usage) {
    if let Some(n) = u["input_tokens"].as_i64() {
        usage.input = n;
    }
    if let Some(n) = u["output_tokens"].as_i64() {
        usage.output = n;
    }
    if let Some(n) = u["cache_read_input_tokens"].as_i64() {
        usage.cache_read = n;
    }
    if let Some(n) = u["cache_creation_input_tokens"].as_i64() {
        usage.cache_write = n;
    }
    if let Some(n) = u["server_tool_use"]["web_search_requests"].as_i64() {
        usage.searches = n;
    }
}

pub async fn list_models(http: &reqwest::Client, ep: &Endpoint) -> Result<Vec<Discovered>, LlmError> {
    let resp = super::send(|| request(http, ep, "/v1/models?limit=1000"), &CancellationToken::new(), ep.timeout_secs).await?;
    let v: Value = resp.json().await.map_err(|e| LlmError::Http(e.to_string()))?;
    Ok(v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m["id"].as_str()?.to_string();
            Some(Discovered {
                display_name: m["display_name"].as_str().unwrap_or(&id).to_string(),
                model: id,
                context_window: m["max_input_tokens"].as_i64(),
                max_output_tokens: m["max_tokens"].as_i64(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_of_one_role_are_merged_and_raw_turns_replayed() {
        let msgs = vec![
            Msg::user("hi"),
            Msg::AnthropicRaw {
                messages: vec![json!({"role": "assistant", "content": [{"type": "thinking", "thinking": "x", "signature": "s"}, {"type": "text", "text": "hello"}]})],
                fallback_text: "hello".into(),
            },
            Msg::user("a"),
            Msg::Plain { role: Role::User, content: vec![Block::Text("b".into()), Block::Text("  ".into())] },
        ];
        let out = messages_json(&msgs);
        assert_eq!(out.len(), 3);
        assert_eq!(out[1]["content"][0]["signature"], "s");
        assert_eq!(out[2]["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn request_body() {
        let req = Request {
            model: "claude-opus-5-5".into(),
            system: Some("be brief".into()),
            messages: vec![Msg::user("hi")],
            thinking: true,
            effort: Some("high".into()),
            temperature: Some(0.2),
            native_search: Some(super::super::NativeSearch { max_uses: 3, blocked_domains: vec![] }),
            ..Default::default()
        };
        let b = body(&req);
        assert_eq!(b["fallbacks"], "default");
        assert_eq!(b["output_config"]["effort"], "high");
        assert!(b.get("temperature").is_none(), "the 2026-10-21 API rejects sampling parameters");
        assert_eq!(b["tools"][0]["type"], "web_search_20260209");
        assert_eq!(b["thinking"]["display"], "summarized");
    }
}
