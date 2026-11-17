//! OpenAI-compatible API for people's own tools and scripts: `GET /api/v1/models` and
//! `POST /api/v1/chat/completions` with a personal API token (`Authorization: Bearer agt_…`).
//! The same model access, budgets, rate limits, usage records and audit trail apply as in chats.

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::response::sse::{Event as SseEvent, Sse};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::app::AppState;
use crate::audit;
use crate::auth::{Client, CurrentUser};
use crate::catalog::ModelRow;
use crate::error::{ApiError, ApiResult};
use crate::llm::{self, Block, Event, Msg, Role, Stop};

pub async fn models(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> ApiResult<Json<Value>> {
    let all = crate::catalog::accessible(&state, &user).await?;
    Ok(Json(json!({
        "object": "list",
        "data": all.iter().map(|m| json!({ "id": crate::catalog::api_name(m, &all), "object": "model", "created": 0, "owned_by": m.provider_name, "name": m.display_name })).collect::<Vec<_>>(),
    })))
}

fn parse_messages(v: &Value) -> ApiResult<(Option<String>, Vec<Msg>)> {
    let arr = v.as_array().ok_or_else(|| ApiError::bad("messages must be an array"))?;
    let mut system: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for m in arr {
        let role = m["role"].as_str().unwrap_or_default();
        let mut blocks = Vec::new();
        match &m["content"] {
            Value::String(s) => blocks.push(Block::Text(s.clone())),
            Value::Array(parts) => {
                for p in parts {
                    match p["type"].as_str() {
                        Some("text") => blocks.push(Block::Text(p["text"].as_str().unwrap_or_default().to_string())),
                        Some("image_url") => {
                            let url = p["image_url"]["url"].as_str().unwrap_or_default();
                            let Some((meta, data)) = url.strip_prefix("data:").and_then(|u| u.split_once(";base64,")) else {
                                return Err(ApiError::bad("images must be data: URLs (base64)"));
                            };
                            blocks.push(Block::Image { media_type: meta.to_string(), data: data.to_string() });
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        match role {
            "system" | "developer" => system.extend(blocks.into_iter().filter_map(|b| if let Block::Text(t) = b { Some(t) } else { None })),
            "user" => out.push(Msg::Plain { role: Role::User, content: blocks }),
            "assistant" => out.push(Msg::Plain { role: Role::Assistant, content: blocks }),
            "tool" => out.push(Msg::user(format!("Tool result:\n{}", m["content"].as_str().unwrap_or_default()))),
            other => return Err(ApiError::bad(format!("unsupported role {other:?}"))),
        }
    }
    if out.is_empty() {
        return Err(ApiError::bad("messages has no user message"));
    }
    Ok(((!system.is_empty()).then(|| system.join("\n\n")), out))
}

fn last_user_text(msgs: &[Msg]) -> String {
    msgs.iter()
        .rev()
        .find_map(|m| match m {
            Msg::Plain { role: Role::User, content } => {
                Some(content.iter().filter_map(|b| if let Block::Text(t) = b { Some(t.as_str()) } else { None }).collect::<Vec<_>>().join("\n"))
            }
            _ => None,
        })
        .unwrap_or_default()
}

fn finish_reason(s: &Stop) -> &'static str {
    match s {
        Stop::MaxTokens => "length",
        Stop::Refusal => "content_filter",
        _ => "stop",
    }
}

pub async fn completions(State(state): State<AppState>, CurrentUser(user): CurrentUser, client: Client, Json(body): Json<Value>) -> ApiResult<Response> {
    let name = body["model"].as_str().ok_or_else(|| ApiError::bad("model is required"))?;
    let model = crate::catalog::for_user_by_name(&state, &user, name).await?;
    let (system, messages) = parse_messages(&body["messages"])?;
    let settings = crate::settings::load(&state).await?;
    crate::usage::check(&state, &user, &settings).await?;
    let mut sys_parts: Vec<String> = Vec::new();
    if let Some(s) = settings.system_prompt.as_deref().filter(|s| !s.trim().is_empty()) {
        sys_parts.push(s.trim().to_string());
    }
    if let Some(s) = system {
        sys_parts.push(s);
    }
    let req = llm::Request {
        model: model.model.clone(),
        system: (!sys_parts.is_empty()).then(|| sys_parts.join("\n\n")),
        messages,
        effort: body["reasoning_effort"].as_str().map(String::from).or_else(|| model.param_str("effort")),
        temperature: body["temperature"].as_f64().or(model.params["temperature"].as_f64()),
        max_tokens: body["max_completion_tokens"].as_u64().or(body["max_tokens"].as_u64()).map(|n| n as u32).or(model.max_output_tokens.map(|n| n as u32)),
        thinking: false,
        ..Default::default()
    };
    let prompt = last_user_text(&req.messages);
    let full = settings.audit_content == "full";
    let ep = crate::catalog::endpoint(&state, model.provider_id).await?;
    let stream = body["stream"].as_bool().unwrap_or(false);
    let include_usage = body["stream_options"]["include_usage"].as_bool().unwrap_or(false);
    let id = format!("chatcmpl-{}", uuid::Uuid::new_v4().simple());
    let created = chrono::Utc::now().timestamp();
    let api_name = name.to_string();
    let job = ApiJob { state: state.clone(), user, client, model, prompt, full };
    if !stream {
        let started = Instant::now();
        let res = llm::stream(&llm::client(state.config.model_timeout_secs), &ep, &req, &CancellationToken::new(), &mut |_| {}).await;
        let o = job.record(&res, started).await;
        let o = o.map_err(|e| ApiError::BadRequest(format!("the model failed: {e}")))?;
        return Ok(Json(json!({
            "id": id, "object": "chat.completion", "created": created, "model": api_name,
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": o.text }, "finish_reason": finish_reason(&o.stop) }],
            "usage": usage_json(&o.usage),
        }))
        .into_response());
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let cancel = CancellationToken::new();
    {
        let cancel = cancel.clone();
        let http = llm::client(state.config.model_timeout_secs);
        tokio::spawn(async move {
            let started = Instant::now();
            let t2 = tx.clone();
            let c2 = cancel.clone();
            let mut on_event = move |e: Event| {
                let delta = match e {
                    Event::Text(t) => json!({ "content": t }),
                    Event::Reasoning(r) => json!({ "reasoning_content": r }),
                    _ => return,
                };
                if t2.send(json!({ "delta": delta })).is_err() {
                    // the client went away
                    c2.cancel();
                }
            };
            let res = llm::stream(&http, &ep, &req, &cancel, &mut on_event).await;
            let o = job.record(&res, started).await;
            let _ = match o {
                Ok(o) => tx.send(json!({ "finish": finish_reason(&o.stop), "usage": usage_json(&o.usage) })),
                Err(e) => tx.send(json!({ "error": e.to_string() })),
            };
        });
    }
    let s = async_stream::stream! {
        let chunk = |delta: Value, finish: Value| json!({
            "id": id, "object": "chat.completion.chunk", "created": created, "model": api_name,
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
        });
        yield Ok::<_, std::convert::Infallible>(SseEvent::default().data(chunk(json!({ "role": "assistant", "content": "" }), Value::Null).to_string()));
        while let Some(ev) = rx.recv().await {
            if let Some(d) = ev.get("delta") {
                yield Ok(SseEvent::default().data(chunk(d.clone(), Value::Null).to_string()));
            } else if let Some(f) = ev.get("finish") {
                yield Ok(SseEvent::default().data(chunk(json!({}), f.clone()).to_string()));
                if include_usage {
                    yield Ok(SseEvent::default().data(json!({ "id": id, "object": "chat.completion.chunk", "created": created, "model": api_name, "choices": [], "usage": ev["usage"] }).to_string()));
                }
                break;
            } else if let Some(e) = ev.get("error") {
                yield Ok(SseEvent::default().data(json!({ "error": { "message": e, "type": "upstream_error" } }).to_string()));
                break;
            }
        }
        yield Ok(SseEvent::default().data("[DONE]"));
    };
    Ok(Sse::new(s).into_response())
}

fn usage_json(u: &llm::Usage) -> Value {
    let prompt = u.input + u.cache_read + u.cache_write;
    json!({ "prompt_tokens": prompt, "completion_tokens": u.output, "total_tokens": prompt + u.output, "prompt_tokens_details": { "cached_tokens": u.cache_read } })
}

struct ApiJob {
    state: AppState,
    user: crate::auth::User,
    client: Client,
    model: ModelRow,
    prompt: String,
    full: bool,
}

impl ApiJob {
    /// Records usage and audits the call.
    async fn record(&self, res: &Result<llm::Outcome, llm::LlmError>, started: Instant) -> Result<llm::Outcome, String> {
        let (usage, status, error) = match res {
            Ok(o) => (o.usage, "ok", None),
            Err(e) => (Default::default(), "error", Some(e.to_string())),
        };
        let cost = crate::usage::record(
            &self.state,
            crate::usage::Record {
                user: Some(self.user.id),
                chat: None,
                message: None,
                model: &self.model,
                source: "api",
                usage,
                latency_ms: started.elapsed().as_millis() as i64,
                status,
                error: error.clone(),
            },
        )
        .await;
        let answer = res.as_ref().map(|o| o.text.clone()).unwrap_or_default();
        audit::log(
            &self.state,
            Some(&self.user),
            Some(&self.client),
            "api.completion",
            Some(("model", self.model.id.to_string())),
            json!({
                "model": self.model.model, "status": status, "error": error, "inputTokens": usage.input + usage.cache_read + usage.cache_write,
                "outputTokens": usage.output, "costUsd": cost, "promptChars": self.prompt.chars().count(), "answerChars": answer.chars().count(),
                "prompt": self.full.then_some(&self.prompt), "answer": self.full.then_some(&answer),
            }),
        )
        .await;
        match res {
            Ok(o) => Ok(o.clone()),
            Err(e) => Err(e.to_string()),
        }
    }
}
