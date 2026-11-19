//! Models that answer through Claude Code: the server runs `claude -p` for an answer and
//! relays what it streams. Claude Code brings its own agent loop (its context handling,
//! sub-agents, planning); it gets none of its built-in tools that touch files, run commands
//! or reach the web. Its tools are Agora's, offered over MCP by this server for the length of
//! the answer: web search and page fetches, the organization's knowledge, and the tools on the
//! person's computer through their desktop app. So grants, approvals and the records of every
//! call are the same as with any other model.
//!
//! A chat continues the Claude Code session of the answer before (`--resume` with
//! `--fork-session`, so every answer has a session of its own and branches work); a chat
//! that has none yet starts one with the conversation so far as text.

use std::process::Stdio;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use super::*;

/// The name Claude Code knows our MCP server by (its tools are `mcp__agora__<name>`).
const SERVER: &str = "agora";
/// Claude Code's own sub-agent tool: the only built-in one it keeps.
const SUBAGENT_TOOL: &str = "Task";

/// An answer Claude Code is writing: what its tool calls need.
pub struct McpRun {
    job: Arc<Job>,
    hub: Arc<Hub>,
    cancel: CancellationToken,
    http: reqwest::Client,
    ep: llm::Endpoint,
    sources: Vec<crate::connectors::Linked>,
    search_key: Option<String>,
    tools: Vec<ToolDef>,
    /// tool calls left for this answer
    budget: Mutex<usize>,
    agent_req: llm::Request,
    spent: Mutex<(llm::Usage, f64)>,
    media: Mutex<HashMap<usize, Vec<llm::Media>>>,
}

/// `POST /api/mcp/{token}`: the MCP server (streamable HTTP, plain JSON answers) of one
/// running answer. The token in the path is the credential: random, known only to the
/// `claude` process started for that answer, and gone when the answer ends.
pub async fn mcp_endpoint(State(state): State<AppState>, Path(token): Path<String>, body: axum::body::Bytes) -> Response {
    let Some(run) = state.mcp_runs.get(&token).map(|r| r.clone()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(req) = serde_json::from_slice::<Value>(&body) else {
        return (StatusCode::BAD_REQUEST, "not JSON").into_response();
    };
    // notifications get no answer
    let Some(id) = req.get("id").filter(|i| !i.is_null()).cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let result = match req["method"].as_str().unwrap_or_default() {
        "initialize" => Ok(json!({
            "protocolVersion": req["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "Agora", "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": run.tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "inputSchema": t.schema })).collect::<Vec<_>>()
        })),
        "tools/call" => Ok(call_tool(&run, &req["params"]).await),
        other => Err(json!({ "code": -32601, "message": format!("method {other} isn't supported") })),
    };
    let answer = match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    };
    axum::Json(answer).into_response()
}

/// Runs one of Agora's tools for Claude Code, as the tool loop would for any other model.
async fn call_tool(run: &McpRun, params: &Value) -> Value {
    let text = |text: &str, is_error: bool| json!({ "content": [{ "type": "text", "text": text }], "isError": is_error });
    let name = params["name"].as_str().unwrap_or_default();
    if !run.tools.iter().any(|t| t.name == name) {
        return text(&format!("Unknown tool {name}."), true);
    }
    if run.cancel.is_cancelled() {
        return text("Stopped by the user.", true);
    }
    {
        let mut left = run.budget.lock().expect("budget");
        if *left == 0 {
            return text(LIMIT_REACHED, true);
        }
        *left -= 1;
    }
    let call = ToolCall {
        id: new_id().to_string(),
        name: name.to_string(),
        input: if params["arguments"].is_object() { params["arguments"].clone() } else { json!({}) },
    };
    let tooling = Tooling {
        http: &run.http,
        ep: &run.ep,
        agent: &run.agent_req,
        agent_calls: 0,
        search_key: run.search_key.as_deref(),
        sources: &run.sources,
        cancel: &run.cancel,
        media: &run.media,
        spent: &run.spent,
    };
    let (content, is_error) = run_tool(&run.job, &run.hub, &call, &tooling, None).await;
    let mut blocks = vec![json!({ "type": "text", "text": if content.is_empty() { "(no output)" } else { content.as_str() } })];
    for m in tooling.returned(&call) {
        if m.is_image() {
            blocks.push(json!({ "type": "image", "data": m.data, "mimeType": m.media_type }));
        } else {
            blocks.push(json!({ "type": "text", "text": format!("[{} can't be passed on here: only images can.]", m.name) }));
        }
    }
    json!({ "content": blocks, "isError": is_error })
}

/// How one run of `claude` ended.
#[derive(Default)]
struct Ran {
    session: Option<String>,
    usage: llm::Usage,
    reported_cost: f64,
    model: String,
    /// why it failed
    error: Option<String>,
    /// text or tool calls came: the run isn't repeated
    produced: bool,
}

/// Where the server reaches itself (the MCP endpoint `claude` connects to).
fn own_url(state: &AppState) -> String {
    let bind = state.config.bind;
    let host = if bind.ip().is_unspecified() {
        "127.0.0.1".to_string()
    } else if bind.is_ipv6() {
        format!("[{}]", bind.ip())
    } else {
        bind.ip().to_string()
    };
    format!("http://{host}:{}", bind.port())
}

/// The conversation before the message being answered, as text (for a session that starts
/// in the middle of a chat).
fn transcript(messages: &[Msg]) -> Option<String> {
    let mut s = String::new();
    for m in messages {
        let (who, text) = match m {
            Msg::Plain { role, content } => (
                if *role == Role::User { "User" } else { "Assistant" },
                content.iter().filter_map(|b| if let Block::Text(t) = b { Some(t.as_str()) } else { None }).collect::<Vec<_>>().join("\n"),
            ),
            Msg::AnthropicRaw { fallback_text, .. } => ("Assistant", fallback_text.clone()),
        };
        if !text.trim().is_empty() {
            s.push_str(&format!("{who}: {}\n\n", text.trim()));
        }
    }
    if s.is_empty() {
        return None;
    }
    // the end matters most
    let n = s.chars().count();
    let s = if n > 200_000 { format!("[…]{}", s.chars().skip(n - 200_000).collect::<String>()) } else { s };
    Some(format!("<conversation-so-far>\n{}\n</conversation-so-far>\n\nThe user's next message:", s.trim()))
}

/// Answers `job` through Claude Code.
pub(super) async fn run(job: Job, hub: Arc<Hub>, cancel: CancellationToken) {
    let started = Instant::now();
    let state = job.state.clone();
    let job = Arc::new(job);
    let fail = |msg: String| Outcome { status: "error", error: Some(msg), ..Default::default() };

    // Agora's tools for this answer
    let ws = &job.settings.web_search;
    let can = job.model.cap("tools");
    let engine = job.web && can && ws.engine != "none";
    let mut tools = tool_defs(engine, engine && ws.fetch_enabled);
    let sources = if job.knowledge && can { crate::connectors::linked(&state, job.user.id).await.unwrap_or_default() } else { vec![] };
    tools.extend(sources.iter().map(crate::connectors::tool_def));
    let desktop = job.desktop.as_ref().filter(|_| can);
    if let Some(d) = desktop {
        let local: Vec<_> = d.tools.iter().filter(|t| !tools.iter().any(|x| x.name == t.def.name)).map(|t| t.def.clone()).collect();
        tools.extend(local);
    }
    let max_calls = match desktop {
        Some(d) => d.policy.general.max_tool_calls.clamp(1, 500),
        None => ws.max_tool_calls.max(1),
    } as usize;
    let subagents = desktop.is_some_and(|d| d.policy.general.subagents);
    let tool_secs = desktop.map_or(600, |d| d.policy.general.tool_timeout_secs.clamp(10, 86_400));
    let system = system_prompt(&job, engine, &sources);

    let flushing = CancellationToken::new();
    {
        let (state, hub, stop, id) = (state.clone(), hub.clone(), flushing.clone(), job.assistant_msg);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.cancelled() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(1500)) => {}
                }
                let (content, parts) = hub.snapshot();
                let _ = sqlx::query("UPDATE messages SET content = $2, parts = $3 WHERE id = $1 AND status = 'streaming'")
                    .bind(id)
                    .bind(content)
                    .bind(Value::Array(parts))
                    .execute(&state.db)
                    .await;
            }
        });
    }
    let done = |out: Outcome| async {
        flushing.cancel();
        finish(&job, &hub, out, started).await;
    };

    let ep = match crate::catalog::endpoint(&state, job.model.provider_id).await {
        Ok(ep) => ep,
        Err(e) => return done(fail(format!("the model's provider isn't usable: {e}"))).await,
    };
    let messages = match history(&state, job.user_msg).await {
        Ok(m) if !m.is_empty() => m,
        Ok(_) => return done(fail("the conversation is empty".into())).await,
        Err(e) => return done(fail(format!("couldn't load the conversation: {e}"))).await,
    };
    // the session of the answer this message follows
    let resume: Option<String> = sqlx::query_scalar(
        "WITH RECURSIVE path AS (
            SELECT id, parent_id, role, raw, 0 AS depth FROM messages WHERE id = $1
            UNION ALL
            SELECT m.id, m.parent_id, m.role, m.raw, path.depth + 1 FROM messages m JOIN path ON m.id = path.parent_id)
         SELECT raw->'claudeCode'->>'sessionId' FROM path WHERE role = 'assistant' AND depth > 0 ORDER BY depth LIMIT 1",
    )
    .bind(job.user_msg)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .flatten();
    let (earlier, last) = messages.split_at(messages.len() - 1);
    let question: Vec<Value> = llm::anthropic::messages_json(last).first().and_then(|m| m["content"].as_array().cloned()).unwrap_or_default();

    let token = crate::auth::random_token();
    let search_key = ws.api_key_enc.as_deref().and_then(base64_decode).and_then(|k| state.secrets.decrypt(&k).ok());
    let run = Arc::new(McpRun {
        job: job.clone(),
        hub: hub.clone(),
        cancel: cancel.clone(),
        http: llm::client(state.config.model_timeout_secs),
        ep: ep.clone(),
        sources,
        search_key,
        tools,
        budget: Mutex::new(max_calls),
        agent_req: llm::Request::default(),
        spent: Mutex::new((llm::Usage::default(), 0.0)),
        media: Mutex::new(HashMap::new()),
    });
    state.mcp_runs.insert(token.clone(), run.clone());

    let dir = state.config.claude_dir.join(job.chat_id.to_string());
    let mut out = Outcome { status: "complete", ..Default::default() };
    let mut ran = Ran::default();
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        ran.error = Some(format!("the folder for Claude Code's sessions ({}) can't be created: {e}", dir.display()));
    } else {
        // a session that is gone (another server, cleaned up) is replaced by a new one that is told the conversation
        for session in [resume.as_deref(), None].into_iter().take(if resume.is_some() { 2 } else { 1 }) {
            let mut content = question.clone();
            if session.is_none()
                && let Some(t) = transcript(earlier)
            {
                content.insert(0, json!({ "type": "text", "text": t }));
            }
            let spec = Spec { dir: &dir, token: &token, system: &system, session, content, subagents, tool_secs, has_tools: !run.tools.is_empty() };
            ran = claude(&job, &hub, &ep, &cancel, spec).await;
            if ran.error.is_none() || ran.produced || cancel.is_cancelled() {
                break;
            }
        }
    }
    state.mcp_runs.remove(&token);

    let cost = crate::usage::record(
        &state,
        crate::usage::Record {
            user: Some(job.user.id),
            chat: Some(job.chat_id),
            message: Some(job.assistant_msg),
            model: &job.model,
            source: "chat",
            usage: ran.usage,
            latency_ms: started.elapsed().as_millis() as i64,
            status: if cancel.is_cancelled() {
                "stopped"
            } else if ran.error.is_some() {
                "error"
            } else {
                "ok"
            },
            error: ran.error.clone().filter(|_| !cancel.is_cancelled()),
        },
    )
    .await;
    out.usage = ran.usage;
    // without prices on the model, what Claude Code itself reports
    out.cost = if cost > 0.0 { cost } else { ran.reported_cost };
    out.model_name = ran.model;
    if cancel.is_cancelled() {
        out.status = "stopped";
    } else if let Some(e) = ran.error {
        tracing::warn!(model = %job.model.model, "claude code failed: {e}");
        out.status = "error";
        out.error = Some(e);
    }
    if out.status == "complete"
        && let Some(sid) = ran.session
    {
        out.raw = Some(json!({ "claudeCode": { "sessionId": sid } }));
    }
    done(out).await;
}

/// What one run of `claude` is given.
struct Spec<'a> {
    dir: &'a std::path::Path,
    token: &'a str,
    system: &'a str,
    session: Option<&'a str>,
    content: Vec<Value>,
    subagents: bool,
    tool_secs: u64,
    has_tools: bool,
}

/// Runs `claude -p` once and relays what it streams to the chat.
async fn claude(job: &Job, hub: &Hub, ep: &llm::Endpoint, cancel: &CancellationToken, spec: Spec<'_>) -> Ran {
    let state = &job.state;
    let mut ran = Ran::default();
    let mcp = json!({ "mcpServers": { SERVER: { "type": "http", "url": format!("{}/api/mcp/{}", own_url(state), spec.token) } } });
    let mut cmd = tokio::process::Command::new(&state.config.claude_bin);
    cmd.current_dir(spec.dir)
        .args(["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--input-format", "stream-json"])
        // none of its own tools for files, commands or the web: only ours (and its sub-agents, when granted)
        .args(["--tools", if spec.subagents { SUBAGENT_TOOL } else { "" }])
        .args(["--strict-mcp-config", "--mcp-config", &mcp.to_string()])
        .args(["--allowedTools", &format!("mcp__{SERVER}")])
        .args(["--permission-prompts", "none"])
        // nothing of the server account's own Claude Code setup: its settings, hooks, commands
        .args(["--setting-sources", "", "--disable-slash-commands"])
        .args(["--system-prompt-snapshot", "off", "--append-system-prompt", spec.system])
        .args(["--model", &job.model.model]);
    if let Some(s) = spec.session {
        cmd.args(["--resume", s, "--fork-session"]);
    }
    if let Some(e) = job.effort.as_deref().or(job.model.param_str("effort").as_deref()).filter(|e| matches!(*e, "low" | "medium" | "high" | "xhigh" | "max")) {
        cmd.args(["--effort", e]);
    }
    cmd.env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .env("DISABLE_AUTOUPDATER", "1")
        .env("MCP_TOOL_TIMEOUT", ((spec.tool_secs + 120) * 1000).to_string())
        .env("MAX_MCP_OUTPUT_TOKENS", "60000");
    // with a key the provider's account pays; without one, the account `claude` is signed in with on this server
    if let Some(key) = ep.api_key.as_deref().filter(|k| !k.is_empty()) {
        cmd.env("ANTHROPIC_API_KEY", key);
    }
    if ep.base_url.starts_with("http") && ep.base_url.trim_end_matches('/') != llm::anthropic::DEFAULT_URL {
        cmd.env("ANTHROPIC_BASE_URL", &ep.base_url);
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            ran.error = Some(if e.kind() == std::io::ErrorKind::NotFound {
                format!("Claude Code isn't installed on the server (`{}` wasn't found; see AGORA_CLAUDE_BIN)", state.config.claude_bin)
            } else {
                format!("Claude Code couldn't be started: {e}")
            });
            return ran;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let line = json!({ "type": "user", "message": { "role": "user", "content": spec.content } }).to_string() + "\n";
        let _ = stdin.write_all(line.as_bytes()).await;
    }
    let errors = child.stderr.take().map(|mut e| {
        tokio::spawn(async move {
            let mut text = String::new();
            let _ = (&mut e).take(20_000).read_to_string(&mut text).await;
            text
        })
    });
    let mut lines = BufReader::new(child.stdout.take().expect("stdout is piped")).lines();
    // its sub-agents, by the id of the call that started them
    let mut agents: HashMap<String, (String, String)> = HashMap::new();
    let mut wrote = false;
    let mut ended = false;
    loop {
        let line = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.start_kill();
                break;
            }
            l = lines.next_line() => match l {
                Ok(Some(l)) => l,
                _ => break,
            },
        };
        let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
        let main = ev["parent_tool_use_id"].is_null();
        match ev["type"].as_str().unwrap_or_default() {
            "system" if ev["subtype"] == "init" => {
                ran.session = ev["session_id"].as_str().map(String::from);
                ran.model = ev["model"].as_str().unwrap_or_default().to_string();
                let connected = ev["mcp_servers"].as_array().is_some_and(|s| s.iter().any(|m| m["name"] == SERVER && m["status"] == "connected"));
                if spec.has_tools && !connected {
                    ran.error = Some(format!("Claude Code couldn't reach Agora's tools at {} (the server must be reachable from itself)", own_url(state)));
                    let _ = child.start_kill();
                    break;
                }
            }
            "stream_event" if main => {
                let e = &ev["event"];
                match e["type"].as_str().unwrap_or_default() {
                    // the text of consecutive turns is set apart
                    "content_block_start" if e["content_block"]["type"] == "text" && wrote => {
                        hub.publish(json!({ "type": "text", "d": "\n\n" }));
                    }
                    "content_block_delta" => match e["delta"]["type"].as_str().unwrap_or_default() {
                        "text_delta" => {
                            let d = e["delta"]["text"].as_str().unwrap_or_default();
                            if !d.is_empty() {
                                hub.publish(json!({ "type": "text", "d": d }));
                                wrote = true;
                                ran.produced = true;
                            }
                        }
                        "thinking_delta" => {
                            let d = e["delta"]["thinking"].as_str().unwrap_or_default();
                            if !d.is_empty() {
                                hub.publish(json!({ "type": "reasoning", "d": d }));
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            "assistant" => {
                for b in ev["message"]["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "tool_use") {
                    ran.produced = true;
                    if b["name"] == SUBAGENT_TOOL
                        && let Some(id) = b["id"].as_str()
                    {
                        let description: String = b["input"]["description"].as_str().unwrap_or("Sub-agent").chars().take(120).collect();
                        let prompt: String = b["input"]["prompt"].as_str().unwrap_or_default().chars().take(4000).collect();
                        hub.publish(json!({ "type": "part", "part": {
                            "type": "agent", "id": id, "description": description, "prompt": prompt, "status": "running", "steps": 0, "output": "" } }));
                        agents.insert(id.to_string(), (description, prompt));
                    }
                }
            }
            "user" => {
                for b in ev["message"]["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "tool_result") {
                    let Some((id, (description, prompt))) = b["tool_use_id"].as_str().and_then(|id| agents.remove(id).map(|a| (id, a))) else { continue };
                    let text = match &b["content"] {
                        Value::String(s) => s.clone(),
                        Value::Array(a) => a.iter().filter_map(|x| x["text"].as_str()).collect::<Vec<_>>().join("\n"),
                        _ => String::new(),
                    };
                    let failed = b["is_error"].as_bool().unwrap_or(false);
                    hub.publish(json!({ "type": "part", "part": {
                        "type": "agent", "id": id, "description": description, "prompt": prompt, "status": if failed { "error" } else { "done" }, "steps": 0,
                        "output": crate::desktop::shown(&text), "error": failed.then_some("The sub-agent failed.") } }));
                }
            }
            "result" => {
                ended = true;
                let u = &ev["usage"];
                let n = |k: &str| u[k].as_i64().unwrap_or(0);
                ran.usage = llm::Usage {
                    input: n("input_tokens"),
                    output: n("output_tokens"),
                    cache_read: n("cache_read_input_tokens"),
                    cache_write: n("cache_creation_input_tokens"),
                    searches: 0,
                };
                ran.reported_cost = ev["total_cost_usd"].as_f64().unwrap_or(0.0);
                if let Some(s) = ev["session_id"].as_str() {
                    ran.session = Some(s.to_string());
                }
                if ev["is_error"].as_bool().unwrap_or(false) || ev["subtype"].as_str().is_some_and(|s| s != "success") {
                    let said = ev["result"].as_str().filter(|s| !s.trim().is_empty()).map(String::from);
                    ran.error = Some(said.unwrap_or_else(|| format!("Claude Code ended with {}", ev["subtype"].as_str().unwrap_or("an error"))));
                }
                break;
            }
            _ => {}
        }
    }
    // sub-agents that never came back
    for (id, (description, prompt)) in agents {
        hub.publish(json!({ "type": "part", "part": {
            "type": "agent", "id": id, "description": description, "prompt": prompt, "status": "stopped", "steps": 0, "output": "" } }));
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;
    let _ = child.start_kill();
    if !ended && !cancel.is_cancelled() && ran.error.is_none() {
        let said = match errors {
            Some(e) => tokio::time::timeout(std::time::Duration::from_secs(2), e).await.ok().and_then(|r| r.ok()).unwrap_or_default(),
            None => String::new(),
        };
        let said = said.trim().lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        ran.error =
            Some(if said.is_empty() { "Claude Code ended without an answer".to_string() } else { format!("Claude Code ended without an answer: {said}") });
    }
    ran
}
