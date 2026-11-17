//! OpenAI-compatible chat completions (`POST {base}/chat/completions`), streaming. Works with
//! OpenAI and with servers that copy its API: Ollama (`http://ollama:11434/v1`), vLLM, llama.cpp,
//! LM Studio, LocalAI, LiteLLM, OpenRouter, Azure-style gateways, …

use std::collections::BTreeMap;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::sse::Reader;
use super::{Block, Discovered, Endpoint, Event, LlmError, Msg, Outcome, Request, Role, Stop, ThinkSplitter, ToolCall, Usage};

fn text_of(blocks: &[Block]) -> String {
    blocks.iter().filter_map(|b| if let Block::Text(t) = b { Some(t.as_str()) } else { None }).collect::<Vec<_>>().join("\n\n")
}

pub fn messages_json(system: Option<&str>, msgs: &[Msg]) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(s) = system.filter(|s| !s.trim().is_empty()) {
        out.push(json!({ "role": "system", "content": s }));
    }
    for m in msgs {
        match m {
            Msg::AnthropicRaw { fallback_text, .. } => out.push(json!({ "role": "assistant", "content": fallback_text })),
            Msg::Plain { role: Role::Assistant, content } => {
                let calls: Vec<Value> = content
                    .iter()
                    .filter_map(|b| match b {
                        Block::ToolUse { id, name, input } => {
                            Some(json!({ "id": id, "type": "function", "function": { "name": name, "arguments": input.to_string() } }))
                        }
                        _ => None,
                    })
                    .collect();
                let mut msg = json!({ "role": "assistant", "content": text_of(content) });
                if !calls.is_empty() {
                    msg["tool_calls"] = json!(calls);
                }
                out.push(msg);
            }
            Msg::Plain { role: Role::User, content } => {
                // tool results are messages of their own
                for b in content {
                    if let Block::ToolResult { id, content, .. } = b {
                        out.push(json!({ "role": "tool", "tool_call_id": id, "content": content }));
                    }
                }
                let images: Vec<&Block> = content.iter().filter(|b| matches!(b, Block::Image { .. })).collect();
                let text = text_of(content);
                if images.is_empty() {
                    if !text.is_empty() {
                        out.push(json!({ "role": "user", "content": text }));
                    }
                } else {
                    let mut parts: Vec<Value> = images
                        .iter()
                        .filter_map(|b| match b {
                            Block::Image { media_type, data } => Some(json!({ "type": "image_url", "image_url": { "url": format!("data:{media_type};base64,{data}") } })),
                            _ => None,
                        })
                        .collect();
                    if !text.is_empty() {
                        parts.push(json!({ "type": "text", "text": text }));
                    }
                    out.push(json!({ "role": "user", "content": parts }));
                }
            }
        }
    }
    out
}

pub fn body(req: &Request) -> Value {
    let mut body = json!({
        "model": req.model,
        "messages": messages_json(req.system.as_deref(), &req.messages),
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(n) = req.max_tokens {
        body["max_tokens"] = json!(n);
    }
    if let Some(e) = req.effort.as_deref().filter(|e| !e.is_empty()) {
        body["reasoning_effort"] = json!(e);
    }
    if !req.tools.is_empty() {
        body["tools"] = json!(
            req.tools
                .iter()
                .map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.schema } }))
                .collect::<Vec<_>>()
        );
    }
    if let Some(Value::Object(extra)) = &req.extra_body {
        for (k, v) in extra {
            body[k] = v.clone();
        }
    }
    body
}

fn request(http: &reqwest::Client, ep: &Endpoint, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
    let base = ep.base_url.trim_end_matches('/');
    let mut r = http.request(method, format!("{base}{path}"));
    if let Some(k) = ep.api_key.as_deref().filter(|k| !k.is_empty()) {
        r = r.bearer_auth(k);
    }
    for (k, v) in &ep.headers {
        r = r.header(k, v);
    }
    r
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    args: String,
}

pub async fn stream(
    http: &reqwest::Client,
    ep: &Endpoint,
    req: &Request,
    cancel: &CancellationToken,
    on_event: &mut (dyn FnMut(Event) + Send),
) -> Result<Outcome, LlmError> {
    let body = serde_json::to_vec(&body(req)).map_err(|e| LlmError::Http(e.to_string()))?;
    let resp = super::send(
        || request(http, ep, reqwest::Method::POST, "/chat/completions").header("content-type", "application/json").body(body.clone()),
        cancel,
        ep.timeout_secs,
    )
    .await?;
    let mut reader = Reader::new(resp, ep.timeout_secs);
    let mut splitter = ThinkSplitter::default();
    let mut text = String::new();
    let mut calls: BTreeMap<u64, PartialCall> = BTreeMap::new();
    let mut usage = Usage::default();
    let mut finish: Option<String> = None;
    let mut model = req.model.clone();
    let mut done = false;
    while let Some(ev) = reader.next(cancel).await? {
        let data = ev.data.trim();
        if data == "[DONE]" {
            done = true;
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
        if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
            let msg = e["message"].as_str().or(e.as_str()).unwrap_or("the model service reported an error").to_string();
            return Err(LlmError::Status { status: 500, message: msg });
        }
        if let Some(m) = v["model"].as_str().filter(|m| !m.is_empty()) {
            model = m.to_string();
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            let prompt = u["prompt_tokens"].as_i64().unwrap_or(0);
            let cached = u["prompt_tokens_details"]["cached_tokens"].as_i64().unwrap_or(0);
            usage.input = (prompt - cached).max(0);
            usage.cache_read = cached;
            usage.output = u["completion_tokens"].as_i64().unwrap_or(0);
        }
        let Some(choice) = v["choices"].get(0) else { continue };
        let d = &choice["delta"];
        // reasoning as a separate field (DeepSeek, vLLM, Ollama, OpenRouter use one of these names)
        for f in ["reasoning_content", "reasoning"] {
            if let Some(r) = d[f].as_str().filter(|r| !r.is_empty()) {
                on_event(Event::Reasoning(r.to_string()));
            }
        }
        if let Some(c) = d["content"].as_str().filter(|c| !c.is_empty()) {
            let (t, r) = splitter.push(c);
            if !r.is_empty() {
                on_event(Event::Reasoning(r));
            }
            if !t.is_empty() {
                text.push_str(&t);
                on_event(Event::Text(t));
            }
        }
        for tc in d["tool_calls"].as_array().into_iter().flatten() {
            let idx = tc["index"].as_u64().unwrap_or(calls.len() as u64);
            let entry = calls.entry(idx).or_default();
            if let Some(id) = tc["id"].as_str().filter(|s| !s.is_empty()) {
                entry.id = id.to_string();
            }
            if let Some(n) = tc["function"]["name"].as_str().filter(|s| !s.is_empty()) {
                if entry.name.is_empty() {
                    on_event(Event::ToolStart { id: entry.id.clone(), name: n.to_string() });
                }
                entry.name.push_str(n);
            }
            if let Some(a) = tc["function"]["arguments"].as_str() {
                entry.args.push_str(a);
            }
        }
        if let Some(f) = choice["finish_reason"].as_str() {
            finish = Some(f.to_string());
        }
    }
    let (t, r) = splitter.finish();
    if !r.is_empty() {
        on_event(Event::Reasoning(r));
    }
    if !t.is_empty() {
        text.push_str(&t);
        on_event(Event::Text(t));
    }
    if finish.is_none() && !done {
        return Err(LlmError::Http("the model's answer broke off".into()));
    }
    // reasoning whose opening tag was part of the prompt template: `…</think>answer`
    if let Some(end) = text.find("</think>")
        && !text[..end].contains("<think>")
    {
        text = text[end + "</think>".len()..].trim_start().to_string();
    }
    let tool_calls: Vec<ToolCall> = calls
        .into_values()
        .filter(|c| !c.name.is_empty())
        .enumerate()
        .map(|(i, c)| ToolCall {
            id: if c.id.is_empty() { format!("call_{i}") } else { c.id },
            name: c.name,
            input: serde_json::from_str(if c.args.trim().is_empty() { "{}" } else { &c.args }).unwrap_or(Value::Null),
        })
        .collect();
    let stop = match finish.as_deref() {
        _ if !tool_calls.is_empty() => Stop::ToolUse,
        Some("stop") | None => Stop::End,
        Some("length") => Stop::MaxTokens,
        Some("content_filter") => Stop::Refusal,
        Some("tool_calls") => Stop::End,
        Some(other) => Stop::Other(other.to_string()),
    };
    Ok(Outcome { text, tool_calls, stop, usage, model, raw_content: None })
}

pub async fn list_models(http: &reqwest::Client, ep: &Endpoint) -> Result<Vec<Discovered>, LlmError> {
    let resp = super::send(|| request(http, ep, reqwest::Method::GET, "/models"), &CancellationToken::new(), ep.timeout_secs).await?;
    let v: Value = resp.json().await.map_err(|e| LlmError::Http(e.to_string()))?;
    let mut out: Vec<Discovered> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m["id"].as_str()?.to_string();
            Some(Discovered {
                display_name: m["name"].as_str().unwrap_or(&id).to_string(),
                model: id,
                context_window: m["context_length"].as_i64().or(m["max_model_len"].as_i64()),
                max_output_tokens: None,
            })
        })
        .collect();
    out.sort_by(|a, b| a.model.cmp(&b.model));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_turns_and_images() {
        let msgs = vec![
            Msg::Plain { role: Role::User, content: vec![Block::Image { media_type: "image/png".into(), data: "AAA".into() }, Block::Text("what?".into())] },
            Msg::Plain {
                role: Role::Assistant,
                content: vec![Block::ToolUse { id: "c1".into(), name: "web_search".into(), input: json!({"query": "x"}) }],
            },
            Msg::Plain { role: Role::User, content: vec![Block::ToolResult { id: "c1".into(), content: "r".into(), is_error: false }] },
            Msg::AnthropicRaw { messages: vec![], fallback_text: "earlier answer".into() },
        ];
        let out = messages_json(Some("sys"), &msgs);
        assert_eq!(out[0]["role"], "system");
        assert_eq!(out[1]["content"][0]["image_url"]["url"], "data:image/png;base64,AAA");
        assert_eq!(out[2]["tool_calls"][0]["function"]["arguments"], "{\"query\":\"x\"}");
        assert_eq!(out[3]["role"], "tool");
        assert_eq!(out[4]["content"], "earlier answer");
    }
}
