//! Answering in a chat: builds the conversation for the model, runs the tool loop (web search,
//! page fetches), streams to the browser, and records everything: the message, usage per model
//! call, web activity and audit events.
//!
//! An answer is generated in a background task. Browsers follow it through a [`Hub`]: closing
//! the tab doesn't lose the answer, and a reopened tab picks the stream up again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::app::{AppState, new_id};
use crate::auth::{Client, User};
use crate::catalog::ModelRow;
use crate::llm::{self, Block, Event, Kind, Msg, Role, SearchHit, Stop, ToolCall, ToolDef};
use crate::settings::Settings;

mod claude_code;
pub use claude_code::{McpRun, mcp_endpoint};

/// An answer being generated on this replica.
pub struct Live {
    pub cancel: CancellationToken,
    pub user_id: Uuid,
    pub hub: Arc<Hub>,
}

/// Fan-out of one answer's events, with a snapshot for late subscribers.
pub struct Hub {
    state: Mutex<HubState>,
    tx: broadcast::Sender<Arc<(u64, Value)>>,
}

#[derive(Default)]
struct HubState {
    seq: u64,
    content: String,
    parts: Vec<Value>,
}

impl Default for Hub {
    fn default() -> Self {
        Hub { state: Mutex::new(HubState::default()), tx: broadcast::channel(4096).0 }
    }
}

impl Hub {
    /// Applies an event to the snapshot and sends it to every subscriber (under one lock, so a
    /// subscriber sees each event exactly once: in its snapshot or in the stream).
    pub fn publish(&self, ev: Value) {
        let mut s = self.state.lock().expect("hub lock");
        match ev["type"].as_str().unwrap_or_default() {
            "text" => s.content.push_str(ev["d"].as_str().unwrap_or_default()),
            "reasoning" => {
                let d = ev["d"].as_str().unwrap_or_default();
                match s.parts.last_mut() {
                    Some(p) if p["type"] == "reasoning" => {
                        let t = p["text"].as_str().unwrap_or_default().to_string() + d;
                        p["text"] = json!(t);
                    }
                    _ => s.parts.push(json!({ "type": "reasoning", "text": d })),
                }
            }
            "part" => {
                let part = ev["part"].clone();
                match s.parts.iter_mut().find(|p| p["id"] == part["id"] && p["type"] == part["type"]) {
                    Some(p) => *p = part,
                    None => s.parts.push(part),
                }
            }
            _ => {}
        }
        s.seq += 1;
        let _ = self.tx.send(Arc::new((s.seq, ev)));
    }

    /// Current content and parts, the last sequence number, and a receiver for what follows.
    pub fn subscribe(&self) -> (Value, u64, broadcast::Receiver<Arc<(u64, Value)>>) {
        let s = self.state.lock().expect("hub lock");
        let rx = self.tx.subscribe();
        (json!({ "type": "snapshot", "content": s.content, "parts": s.parts }), s.seq, rx)
    }

    pub fn snapshot(&self) -> (String, Vec<Value>) {
        let s = self.state.lock().expect("hub lock");
        (s.content.clone(), s.parts.clone())
    }
}

/// An SSE response following a hub: `first` events, then everything published after `seq`.
pub fn sse_stream(
    hub: Arc<Hub>,
    first: Vec<Value>,
    mut seq: u64,
    mut rx: broadcast::Receiver<Arc<(u64, Value)>>,
) -> impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>> {
    async_stream::stream! {
        for ev in first {
            yield Ok(axum::response::sse::Event::default().data(ev.to_string()));
        }
        loop {
            match rx.recv().await {
                Ok(item) => {
                    let (n, ev) = &*item;
                    if *n <= seq {
                        continue;
                    }
                    seq = *n;
                    let end = ev["type"] == "end";
                    yield Ok(axum::response::sse::Event::default().data(ev.to_string()));
                    if end {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // too slow a reader: start over from a fresh snapshot
                    let (snap, s, new_rx) = hub.subscribe();
                    rx = new_rx;
                    seq = s;
                    yield Ok(axum::response::sse::Event::default().data(snap.to_string()));
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

/// Everything one answer needs.
pub struct Job {
    pub state: AppState,
    pub user: User,
    pub client: Client,
    pub settings: Settings,
    pub chat_id: Uuid,
    pub model: ModelRow,
    pub user_msg: Uuid,
    pub assistant_msg: Uuid,
    pub web: bool,
    /// let the model search the person's connected knowledge sources
    pub knowledge: bool,
    pub chat_system: Option<String>,
    /// the chat's project: its instructions and files
    pub project: Option<crate::routes::projects::Context>,
    pub effort: Option<String>,
    /// write a title after this answer
    pub title_after: bool,
    /// tools on the person's computer (the request came from their desktop app)
    pub desktop: Option<crate::desktop::Session>,
    /// `compact`: summarize the conversation instead of answering
    pub command: Option<String>,
}

/// Starts generating; returns the hub to stream from.
pub fn start(job: Job) -> (Arc<Hub>, Value, u64, broadcast::Receiver<Arc<(u64, Value)>>) {
    let hub = Arc::new(Hub::default());
    let cancel = CancellationToken::new();
    job.state.live.insert(job.assistant_msg, Live { cancel: cancel.clone(), user_id: job.user.id, hub: hub.clone() });
    let (snap, seq, rx) = hub.subscribe();
    let h = hub.clone();
    tokio::spawn(async move {
        let state = job.state.clone();
        let id = job.assistant_msg;
        run(job, h.clone(), cancel).await;
        h.publish(json!({ "type": "end" }));
        state.live.remove(&id);
    });
    (hub, snap, seq, rx)
}

const WEB_SEARCH: &str = "web_search";
const FETCH_URL: &str = "fetch_url";
/// Hands a piece of the work to a sub-agent (answers that use the desktop's tools).
const TASK: &str = "task";
const LIMIT_REACHED: &str = "Tool call limit reached for this answer. Answer with the information you already have.";

const AGENT_NOTE: &str = "You are a sub-agent: another instance of the assistant gave you the task below and sees nothing of what you do except your \
    final message. Do the task with your tools, then report: what you found or changed, with file paths and the facts that matter, and what remains \
    open. Be brief and exact; don't ask questions, decide what is reasonable and say what you assumed.";

const COMPACT_PROMPT: &str = "The conversation is about to exceed what fits in your context. Write the summary that will replace everything above \
    (this request included) so that the work can go on without it. Don't call tools. Cover, in this order:\n\
    1. What the user asked for, and their latest request word for word.\n\
    2. What was done: files read and changed (with paths), commands run and what they showed, decisions and why.\n\
    3. Facts learned that are needed again (names, signatures, error messages, values) — exact, not paraphrased.\n\
    4. What the user corrected or declined.\n\
    5. Where the work stands and the next steps.\n\
    Write only the summary.";

fn task_tool() -> ToolDef {
    ToolDef {
        name: TASK.into(),
        description: "Hand a self-contained piece of work to a sub-agent that starts with an empty context: a broad search through the code, a \
            question to investigate, an independent change. It has your tools (except this one), doesn't see this conversation, and returns only its \
            final report, so your own context stays small. Say in the prompt everything it needs to know and what to report. Several task calls in \
            one turn run at the same time; don't let two of them change the same files."
            .into(),
        schema: json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "Three to six words naming the task" },
                "prompt": { "type": "string", "description": "The task, with everything the sub-agent needs to know" }
            },
            "required": ["description", "prompt"],
            "additionalProperties": false
        }),
    }
}

/// What the tool calls of one answer share.
struct Tooling<'a> {
    http: &'a reqwest::Client,
    ep: &'a llm::Endpoint,
    /// what a sub-agent's model calls start from (the answer's request without `task`)
    agent: &'a llm::Request,
    agent_calls: usize,
    search_key: Option<&'a str>,
    sources: &'a [crate::connectors::Linked],
    cancel: &'a CancellationToken,
    /// images and documents a tool call returned, until its result is built (by the call's address)
    media: &'a Mutex<HashMap<usize, Vec<llm::Media>>>,
    /// usage and cost of model calls made inside tools (sub-agents, summaries)
    spent: &'a Mutex<(llm::Usage, f64)>,
}

impl Tooling<'_> {
    /// Takes what the call returned besides text.
    fn returned(&self, call: &ToolCall) -> Vec<llm::Media> {
        self.media.lock().expect("media").remove(&(call as *const ToolCall as usize)).unwrap_or_default()
    }
}

/// One model call that isn't shown as it streams; its usage is recorded. `Err` says why it failed.
async fn quiet_call(job: &Job, t: &Tooling<'_>, req: &llm::Request) -> Result<llm::Outcome, String> {
    let started = Instant::now();
    let result = llm::stream(t.http, t.ep, req, t.cancel, &mut |_e: Event| {}).await;
    let stopped = t.cancel.is_cancelled();
    let (usage, status, error) = match &result {
        Ok(o) => (o.usage, "ok", None),
        Err(_) if stopped => (Default::default(), "stopped", None),
        Err(e) => (Default::default(), "error", Some(e.to_string())),
    };
    let cost = crate::usage::record(
        &job.state,
        crate::usage::Record {
            user: Some(job.user.id),
            chat: Some(job.chat_id),
            message: Some(job.assistant_msg),
            model: &job.model,
            source: "chat",
            usage,
            latency_ms: started.elapsed().as_millis() as i64,
            status,
            error,
        },
    )
    .await;
    {
        let mut spent = t.spent.lock().expect("spent");
        spent.0.add(&usage);
        spent.1 += cost;
    }
    result.map_err(|e| if stopped { "Stopped by the user.".to_string() } else { e.to_string() })
}

/// Roughly how many tokens a conversation takes (on the high side).
fn approx_tokens(messages: &[Msg]) -> i64 {
    let chars: usize = messages
        .iter()
        .map(|m| match m {
            Msg::Plain { content, .. } => content
                .iter()
                .map(|b| match b {
                    Block::Text(t) => t.len(),
                    Block::Image { .. } => 6000,
                    Block::Document { data, .. } => data.len() / 8,
                    Block::ToolUse { input, .. } => input.to_string().len() + 40,
                    Block::ToolResult { content, media, .. } => content.len() + 20 + media.len() * 6000,
                })
                .sum::<usize>(),
            Msg::AnthropicRaw { messages, .. } => messages.iter().map(|m| m.to_string().len()).sum(),
        })
        .sum();
    (chars / 3) as i64
}

/// The tokens a conversation may take before it is summarized: 70% of the model's context
/// window (Claude models without one on record: 200,000 tokens). `None`: never.
fn compact_at(model: &ModelRow) -> Option<i64> {
    let window = model.context_window.map(i64::from).filter(|w| *w >= 4000).or((model.kind() == Kind::Anthropic).then_some(200_000))?;
    Some(window * 7 / 10)
}

/// The message a summary stands in the conversation as, with the user's message it precedes.
fn summary_message(summary: &str, user: Option<&[Block]>) -> Msg {
    // the files of the user's message stay with it
    let mut content: Vec<Block> = user.unwrap_or_default().iter().filter(|b| matches!(b, Block::Image { .. } | Block::Document { .. })).cloned().collect();
    let user = user.map(|blocks| blocks.iter().filter_map(|b| if let Block::Text(t) = b { Some(t.as_str()) } else { None }).collect::<Vec<_>>().join("\n"));
    let mut s =
        format!("[The conversation up to here was summarized to make room. This is the summary; the original messages are gone.]\n\n{}", summary.trim());
    match user {
        Some(u) => s.push_str(&format!("\n\n[End of the summary. The user's next message:]\n\n{u}")),
        None => s.push_str("\n\n[End of the summary. Go on with the work where it stands; don't ask the user to repeat anything.]"),
    }
    content.push(Block::Text(s));
    Msg::Plain { role: Role::User, content }
}

/// Asks the model to summarize `messages` (the conversation so far).
async fn summarize(job: &Job, hub: &Hub, t: &Tooling<'_>, req: &llm::Request, messages: &[Msg]) -> Option<String> {
    let id = new_id().to_string();
    hub.publish(json!({ "type": "part", "part": { "type": "compaction", "id": id, "status": "running" } }));
    let mut req = req.clone();
    req.messages = messages.to_vec();
    req.messages.push(Msg::user(COMPACT_PROMPT));
    let res = quiet_call(job, t, &req).await;
    match res {
        Ok(o) if !o.text.trim().is_empty() => {
            hub.publish(json!({ "type": "part", "part": { "type": "compaction", "id": id, "status": "done", "chars": o.text.chars().count() } }));
            Some(o.text)
        }
        other => {
            let e = other.err().unwrap_or_else(|| "the model wrote no summary".into());
            tracing::warn!(model = %job.model.model, "summarizing the conversation failed: {e}");
            hub.publish(json!({ "type": "part", "part": { "type": "compaction", "id": id, "status": "error", "error": e } }));
            None
        }
    }
}

fn tool_defs(web_engine: bool, fetch: bool) -> Vec<ToolDef> {
    let mut tools = Vec::new();
    if web_engine {
        tools.push(ToolDef {
            name: WEB_SEARCH.into(),
            description:
                "Search the web. Use it for current events, facts you're unsure of, and anything after your training data. Returns titles, URLs and snippets."
                    .into(),
            schema: json!({
                "type": "object",
                "properties": { "query": { "type": "string", "description": "The search query, as you would type it into a search engine" } },
                "required": ["query"],
                "additionalProperties": false
            }),
        });
    }
    if fetch {
        tools.push(ToolDef {
            name: FETCH_URL.into(),
            description: "Open a web page and read its text, e.g. a search result or a URL the user gave.".into(),
            schema: json!({
                "type": "object",
                "properties": { "url": { "type": "string", "description": "The full http(s) URL" } },
                "required": ["url"],
                "additionalProperties": false
            }),
        });
    }
    tools
}

/// The conversation from the root to `leaf` (inclusive), as model messages.
async fn history(state: &AppState, leaf: Uuid) -> anyhow::Result<Vec<Msg>> {
    let rows: Vec<(Uuid, String, String, Option<Value>, String)> = sqlx::query_as(
        "WITH RECURSIVE path AS (
            SELECT id, parent_id, role, content, raw, status, 0 AS depth FROM messages WHERE id = $1
            UNION ALL
            SELECT m.id, m.parent_id, m.role, m.content, m.raw, m.status, path.depth + 1 FROM messages m JOIN path ON m.id = path.parent_id)
         SELECT id, role, content, raw, status FROM path ORDER BY depth DESC",
    )
    .bind(leaf)
    .fetch_all(&state.db)
    .await?;
    // what people attached to their messages
    let ids: Vec<Uuid> = rows.iter().filter(|r| r.1 == "user").map(|r| r.0).collect();
    let files: Vec<(Uuid, String, String, Vec<u8>)> =
        sqlx::query_as("SELECT message_id, name, media_type, data FROM attachments WHERE message_id = ANY($1) ORDER BY created_at, id")
            .bind(&ids)
            .fetch_all(&state.db)
            .await?;
    let mut out = Vec::new();
    let mut last_user: Vec<Block> = Vec::new();
    for (id, role, content, raw, status) in rows {
        if role == "user" {
            use base64::Engine;
            let mut blocks: Vec<Block> = files
                .iter()
                .filter(|f| f.0 == id)
                .map(|f| llm::Media { name: f.1.clone(), media_type: f.2.clone(), data: base64::engine::general_purpose::STANDARD.encode(&f.3) }.block())
                .collect();
            blocks.push(Block::Text(content));
            last_user = blocks.clone();
            out.push(Msg::Plain { role: Role::User, content: blocks });
            continue;
        }
        // an answer during which the conversation was summarized: what came before is replaced
        if status == "complete"
            && let Some(c) = raw.as_ref().map(|r| &r["compact"])
            && let Some(summary) = c["summary"].as_str()
        {
            out.clear();
            out.push(summary_message(summary, c["keepUser"].as_bool().unwrap_or(false).then_some(last_user.as_slice())));
        }
        if content.trim().is_empty() {
            continue;
        }
        match raw.as_ref().and_then(|r| r["anthropic"].as_array()).filter(|_| status == "complete") {
            Some(msgs) => out.push(Msg::AnthropicRaw { messages: msgs.clone(), fallback_text: content }),
            None => out.push(Msg::assistant(content)),
        }
    }
    Ok(out)
}

/// The conversation as the model gets it: the project's images and PDFs come with its first message
/// (whatever was summarized away since).
fn with_project(job: &Job, messages: &[Msg]) -> Vec<Msg> {
    let mut out = messages.to_vec();
    let Some(p) = job.project.as_ref().filter(|p| !p.media.is_empty() && job.model.cap("vision")) else {
        return out;
    };
    let files: Vec<Block> = p.media.iter().map(llm::Media::block).collect();
    match out.first_mut() {
        Some(Msg::Plain { role: Role::User, content }) => {
            content.splice(0..0, files);
        }
        _ => out.insert(0, Msg::Plain { role: Role::User, content: files }),
    }
    out
}

fn system_prompt(job: &Job, tools: bool, sources: &[crate::connectors::Linked]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = job.settings.system_prompt.as_deref().filter(|s| !s.trim().is_empty()) {
        parts.push(s.trim().to_string());
    }
    if let Some(s) = job.model.param_str("systemPrompt") {
        parts.push(s.trim().to_string());
    }
    if let Some(p) = &job.project {
        parts.push(p.system(job.settings.allow_user_system_prompt, job.model.cap("vision")));
    }
    if let Some(s) = job.chat_system.as_deref().filter(|s| !s.trim().is_empty() && job.settings.allow_user_system_prompt) {
        parts.push(s.trim().to_string());
    }
    let mut ctx = format!("Today is {}.", chrono::Utc::now().format("%A, %Y-%m-%d"));
    if tools {
        ctx.push_str(" You can search the web and open pages. When you use information from the web, cite the pages as Markdown links.");
    }
    if !sources.is_empty() {
        let names: Vec<&str> = sources.iter().map(|s| s.connector.name.as_str()).collect();
        ctx.push_str(&format!(
            " You can search the organization's internal knowledge ({}). Search it first for questions about the organization, its projects, processes and people; base your answer on what you find and cite the pages as Markdown links. Say so when it has nothing on the question.",
            names.join(", ")
        ));
    }
    if let Some(d) = job.desktop.as_ref().filter(|_| job.model.cap("tools")) {
        ctx.push_str(&d.system_note());
    }
    parts.push(ctx);
    parts.join("\n\n")
}

struct NativeSearchLog {
    query: String,
    results: Vec<SearchHit>,
    error: Option<String>,
}

async fn run(job: Job, hub: Arc<Hub>, cancel: CancellationToken) {
    let started = Instant::now();
    let state = job.state.clone();
    let http = llm::client(state.config.model_timeout_secs);
    let ws = &job.settings.web_search;
    let kind = job.model.kind();
    if kind == Kind::ClaudeCode {
        return claude_code::run(job, hub, cancel).await;
    }
    let tools_ok = job.web && job.model.cap("tools");
    let native = tools_ok && kind == Kind::Anthropic && ws.claude_native;
    let engine = tools_ok && !native && ws.engine != "none";
    let fetch = tools_ok && ws.fetch_enabled && (engine || native);
    let mut tools = tool_defs(engine, fetch);
    let sources = if job.knowledge && job.model.cap("tools") { crate::connectors::linked(&state, job.user.id).await.unwrap_or_default() } else { vec![] };
    tools.extend(sources.iter().map(crate::connectors::tool_def));
    // tools on the person's computer; ours win when a name is taken
    let desktop = job.desktop.as_ref().filter(|_| job.model.cap("tools"));
    if let Some(d) = desktop {
        let local: Vec<_> = d.tools.iter().filter(|t| !tools.iter().any(|x| x.name == t.def.name)).map(|t| t.def.clone()).collect();
        tools.extend(local);
        if d.policy.general.subagents {
            tools.retain(|t| t.name != TASK);
            tools.push(task_tool());
        }
    }
    let agent_calls = desktop.map_or(0, |d| d.policy.general.max_subagent_tool_calls.clamp(1, 200)) as usize;
    let max_calls = match desktop {
        Some(d) => d.policy.general.max_tool_calls.clamp(1, 500),
        None => ws.max_tool_calls.max(1),
    } as usize;
    let search_key = ws.api_key_enc.as_deref().and_then(base64_decode).and_then(|k| state.secrets.decrypt(&k).ok());

    // keep the stored message current while streaming (other tabs, restarts)
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

    let ep = match crate::catalog::endpoint(&state, job.model.provider_id).await {
        Ok(ep) => ep,
        Err(e) => {
            flushing.cancel();
            finish(&job, &hub, Outcome { status: "error", error: Some(format!("the model's provider isn't usable: {e}")), ..Default::default() }, started)
                .await;
            return;
        }
    };
    let mut messages = match history(&state, job.user_msg).await {
        Ok(m) => m,
        Err(e) => {
            flushing.cancel();
            finish(&job, &hub, Outcome { status: "error", error: Some(format!("couldn't load the conversation: {e}")), ..Default::default() }, started).await;
            return;
        }
    };
    let mut req = llm::Request {
        model: job.model.model.clone(),
        system: Some(system_prompt(&job, engine || native, &sources)),
        messages: vec![],
        tools: tools.clone(),
        native_search: native.then(|| llm::NativeSearch { max_uses: ws.max_tool_calls.max(1), blocked_domains: ws.blocked_domains.clone() }),
        effort: job.effort.clone().or_else(|| job.model.param_str("effort")),
        temperature: job.model.params["temperature"].as_f64(),
        max_tokens: job.model.max_output_tokens.map(|n| n.max(256) as u32),
        thinking: kind == Kind::Anthropic && job.model.cap("reasoning"),
        extra_body: job.model.params.get("extraBody").cloned().filter(|v| v.is_object()),
    };
    let mut out = Outcome { status: "complete", ..Default::default() };
    let mut anthropic_turns: Vec<Value> = Vec::new();
    let mut tool_budget = max_calls;
    let mut text_so_far = String::new();
    let mut agent_req = req.clone();
    agent_req.tools.retain(|t| t.name != TASK);
    agent_req.system = Some(format!("{}\n\n{AGENT_NOTE}", agent_req.system.as_deref().unwrap_or_default()));
    let spent = Mutex::new((llm::Usage::default(), 0.0));
    let media = Mutex::new(HashMap::new());
    let tooling = Tooling {
        http: &http,
        ep: &ep,
        agent: &agent_req,
        agent_calls,
        search_key: search_key.as_deref(),
        sources: &sources,
        cancel: &cancel,
        media: &media,
        spent: &spent,
    };
    // a conversation too long for the model is summarized: before the answer, and again
    // whenever the tool results of this answer fill the context
    let limit = compact_at(&job.model);
    let mut compacted: Option<Value> = None;
    if let Some(limit) = limit
        && messages.len() > 2
        && approx_tokens(&with_project(&job, &messages)) + (req.system.as_deref().map_or(0, str::len) / 3) as i64 > limit
        && let Some(summary) = summarize(&job, &hub, &tooling, &req, &messages[..messages.len() - 1]).await
    {
        let question = match messages.last() {
            Some(Msg::Plain { content, .. }) => content.clone(),
            _ => vec![],
        };
        messages = vec![summary_message(&summary, Some(&question))];
        compacted = Some(json!({ "summary": summary, "keepUser": true }));
    }
    // `/compact`: the person asked for the summary themselves; nothing is answered
    let summarize_only = job.command.as_deref() == Some("compact");
    if summarize_only {
        let said = if messages.len() < 3 {
            "There is nothing to summarize yet.".to_string()
        } else if let Some(summary) = summarize(&job, &hub, &tooling, &req, &messages[..messages.len() - 1]).await {
            let n = summary.chars().count();
            compacted = Some(json!({ "summary": summary, "keepUser": false }));
            format!("The conversation so far was summarized ({n} characters); the next answers start from that summary.")
        } else {
            out.status = "error";
            out.error = Some("The conversation couldn't be summarized.".into());
            String::new()
        };
        hub.publish(json!({ "type": "text", "d": said }));
    }
    for round in 0..(if summarize_only { 0 } else { max_calls + 4 }) {
        if cancel.is_cancelled() {
            out.status = "stopped";
            break;
        }
        // separate the text of consecutive rounds
        if round > 0 && !text_so_far.is_empty() && !text_so_far.ends_with("\n\n") {
            hub.publish(json!({ "type": "text", "d": "\n\n" }));
            text_so_far.push_str("\n\n");
        }
        req.messages = with_project(&job, &messages);
        let mut native_log: HashMap<String, NativeSearchLog> = HashMap::new();
        let call_start = Instant::now();
        let result = {
            let hub = &hub;
            let native_log = &mut native_log;
            let mut on_event = |e: Event| match e {
                Event::Text(d) => hub.publish(json!({ "type": "text", "d": d })),
                Event::Reasoning(d) => hub.publish(json!({ "type": "reasoning", "d": d })),
                Event::ToolStart { .. } => {}
                Event::NativeSearch { id, query } => {
                    hub.publish(json!({ "type": "part", "part": { "type": "search", "id": id, "engine": "anthropic", "query": query, "status": "running" } }));
                    native_log.insert(id, NativeSearchLog { query, results: vec![], error: None });
                }
                Event::NativeSearchResults { id, results, error } => {
                    let query = native_log.get(&id).map(|l| l.query.clone()).unwrap_or_default();
                    hub.publish(json!({ "type": "part", "part": {
                        "type": "search", "id": id, "engine": "anthropic", "query": query,
                        "status": if error.is_some() { "error" } else { "done" }, "error": error, "results": results } }));
                    let entry = native_log.entry(id).or_insert_with(|| NativeSearchLog { query, results: vec![], error: None });
                    entry.results = results;
                    entry.error = error;
                }
            };
            llm::stream(&http, &ep, &req, &cancel, &mut on_event).await
        };
        let latency = call_start.elapsed().as_millis() as i64;
        for (_, log) in native_log {
            record_web(&job, "search", "anthropic", Some(&log.query), None, &log.results, log.error.as_deref(), 0).await;
        }
        let o = match result {
            Ok(o) => o,
            Err(e) => {
                let stopped = cancel.is_cancelled();
                crate::usage::record(
                    &state,
                    crate::usage::Record {
                        user: Some(job.user.id),
                        chat: Some(job.chat_id),
                        message: Some(job.assistant_msg),
                        model: &job.model,
                        source: "chat",
                        usage: Default::default(),
                        latency_ms: latency,
                        status: if stopped { "stopped" } else { "error" },
                        error: (!stopped).then(|| e.to_string()),
                    },
                )
                .await;
                if stopped {
                    out.status = "stopped";
                } else {
                    tracing::warn!(model = %job.model.model, "model call failed: {e}");
                    out.status = "error";
                    out.error = Some(e.to_string());
                }
                break;
            }
        };
        out.cost += crate::usage::record(
            &state,
            crate::usage::Record {
                user: Some(job.user.id),
                chat: Some(job.chat_id),
                message: Some(job.assistant_msg),
                model: &job.model,
                source: "chat",
                usage: o.usage,
                latency_ms: latency,
                status: "ok",
                error: None,
            },
        )
        .await;
        out.usage.add(&o.usage);
        out.model_name = o.model.clone();
        text_so_far.push_str(&o.text);
        if let Some(blocks) = &o.raw_content {
            anthropic_turns.push(json!({ "role": "assistant", "content": blocks }));
        }
        match o.stop {
            Stop::ToolUse if !o.tool_calls.is_empty() => {
                // the model's turn, then the results
                let assistant_turn = match &o.raw_content {
                    Some(blocks) => Msg::AnthropicRaw { messages: vec![json!({ "role": "assistant", "content": blocks })], fallback_text: o.text.clone() },
                    None => {
                        let mut content = vec![Block::Text(o.text.clone())];
                        content.extend(o.tool_calls.iter().map(|c| Block::ToolUse { id: c.id.clone(), name: c.name.clone(), input: c.input.clone() }));
                        Msg::Plain { role: Role::Assistant, content }
                    }
                };
                messages.push(assistant_turn);
                // calls that only read run at the same time; the others one after another
                let mut results = Vec::new();
                let calls = &o.tool_calls;
                let mut i = 0;
                while i < calls.len() {
                    let mut j = i + 1;
                    if reads_only(&job, &sources, &calls[i].name) {
                        while j < calls.len() && reads_only(&job, &sources, &calls[j].name) {
                            j += 1;
                        }
                    }
                    let within: Vec<bool> = calls[i..j]
                        .iter()
                        .map(|_| {
                            tool_budget > 0 && {
                                tool_budget -= 1;
                                true
                            }
                        })
                        .collect();
                    let (job, hub, tooling) = (&job, &*hub, &tooling);
                    let outs =
                        futures::future::join_all(calls[i..j].iter().zip(within).map(|(call, ok)| async move {
                            if ok { run_tool(job, hub, call, tooling, None).await } else { (LIMIT_REACHED.to_string(), true) }
                        }))
                        .await;
                    for (call, (content, is_error)) in calls[i..j].iter().zip(outs) {
                        results.push(Block::ToolResult { id: call.id.clone(), content, is_error, media: tooling.returned(call) });
                    }
                    i = j;
                }
                if kind == Kind::Anthropic {
                    let blocks: Vec<Value> = llm::anthropic::messages_json(&[Msg::Plain { role: Role::User, content: results.clone() }])
                        .into_iter()
                        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
                        .collect();
                    anthropic_turns.push(json!({ "role": "user", "content": blocks }));
                }
                messages.push(Msg::Plain { role: Role::User, content: results });
                let used = (o.usage.input + o.usage.cache_read + o.usage.cache_write + o.usage.output).max(approx_tokens(&messages) * 3 / 4);
                if let Some(limit) = limit
                    && used > limit
                    && !cancel.is_cancelled()
                    && let Some(summary) = summarize(&job, &hub, &tooling, &req, &messages).await
                {
                    messages = vec![summary_message(&summary, None)];
                    anthropic_turns.clear();
                    compacted = Some(json!({ "summary": summary, "keepUser": false }));
                }
                continue;
            }
            Stop::Pause if o.raw_content.is_some() => {
                // a long server-side tool turn: send it back as it is to let Claude continue
                messages.push(Msg::AnthropicRaw { messages: vec![json!({ "role": "assistant", "content": o.raw_content })], fallback_text: o.text.clone() });
                continue;
            }
            Stop::MaxTokens => {
                out.note = Some("The answer was cut off (output limit).".into());
                break;
            }
            Stop::Refusal => {
                out.note = Some("The model declined to answer this request.".into());
                break;
            }
            _ => break,
        }
    }
    flushing.cancel();
    {
        let spent = spent.lock().expect("spent");
        out.usage.add(&spent.0);
        out.cost += spent.1;
    }
    if out.status == "complete" {
        let mut raw = serde_json::Map::new();
        if kind == Kind::Anthropic && !anthropic_turns.is_empty() {
            raw.insert("anthropic".into(), Value::Array(anthropic_turns));
        }
        if let Some(c) = compacted {
            raw.insert("compact".into(), c);
        }
        if !raw.is_empty() {
            out.raw = Some(Value::Object(raw));
        }
    }
    finish(&job, &hub, out, started).await;
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

#[derive(Default)]
struct Outcome {
    status: &'static str,
    error: Option<String>,
    note: Option<String>,
    usage: llm::Usage,
    cost: f64,
    model_name: String,
    raw: Option<Value>,
}

/// Whether a tool only reads (searches, page fetches, reading the project, sub-agents): such
/// calls of one turn run at the same time.
fn reads_only(job: &Job, sources: &[crate::connectors::Linked], name: &str) -> bool {
    if matches!(name, WEB_SEARCH | FETCH_URL | TASK) || sources.iter().any(|s| s.tool == name) {
        return true;
    }
    job.desktop.as_ref().and_then(|d| d.find(name)).is_some_and(|t| {
        t.feature == "code" && matches!(t.remote_name.as_str(), "collect_files" | "list_files" | "read_file" | "search_files" | "command_output")
    })
}

/// A sub-agent: the same model with the same tools and an empty context works on
/// `call.input.prompt`; only its final message goes back. Its tool calls show in the chat
/// under it and are recorded like any other.
fn run_agent<'a>(job: &'a Job, hub: &'a Hub, call: &'a ToolCall, t: &'a Tooling<'a>) -> futures::future::BoxFuture<'a, (String, bool)> {
    Box::pin(async move {
        let prompt = call.input["prompt"].as_str().unwrap_or_default().trim();
        let description: String = call.input["description"].as_str().unwrap_or("Sub-agent").trim().chars().take(120).collect();
        if prompt.is_empty() {
            return ("The prompt is empty.".to_string(), true);
        }
        let id = new_id().to_string();
        let shown: String = prompt.chars().take(4000).collect();
        let part = |status: &str, steps: usize, output: &str, error: Option<&str>| {
            json!({ "type": "part", "part": {
                "type": "agent", "id": id, "description": description, "prompt": shown, "status": status, "steps": steps,
                "output": crate::desktop::shown(output), "error": error } })
        };
        hub.publish(part("running", 0, "", None));
        let kind = job.model.kind();
        let mut req = t.agent.clone();
        let mut messages = vec![Msg::user(prompt)];
        let mut budget = t.agent_calls;
        let mut steps = 0usize;
        let mut answer = String::new();
        for _ in 0..(t.agent_calls + 4) {
            req.messages = with_project(job, &messages);
            let o = match quiet_call(job, t, &req).await {
                Ok(o) => o,
                Err(e) => {
                    hub.publish(part(if t.cancel.is_cancelled() { "stopped" } else { "error" }, steps, "", Some(&e)));
                    return (format!("The sub-agent didn't finish: {e}"), true);
                }
            };
            if !o.text.trim().is_empty() {
                answer = o.text.clone();
            }
            match o.stop {
                Stop::ToolUse if !o.tool_calls.is_empty() => {
                    messages.push(match &o.raw_content {
                        Some(blocks) => Msg::AnthropicRaw { messages: vec![json!({ "role": "assistant", "content": blocks })], fallback_text: o.text.clone() },
                        None => {
                            let mut content = vec![Block::Text(o.text.clone())];
                            content.extend(o.tool_calls.iter().map(|c| Block::ToolUse { id: c.id.clone(), name: c.name.clone(), input: c.input.clone() }));
                            Msg::Plain { role: Role::Assistant, content }
                        }
                    });
                    let mut results = Vec::new();
                    for c in &o.tool_calls {
                        let (content, is_error) = if budget == 0 {
                            ("Tool call limit reached for this task. Report what you have.".to_string(), true)
                        } else {
                            budget -= 1;
                            steps += 1;
                            hub.publish(part("running", steps, "", None));
                            run_tool(job, hub, c, t, Some(&id)).await
                        };
                        results.push(Block::ToolResult { id: c.id.clone(), content, is_error, media: t.returned(c) });
                    }
                    messages.push(Msg::Plain { role: Role::User, content: results });
                }
                Stop::Pause if o.raw_content.is_some() && kind == Kind::Anthropic => {
                    messages
                        .push(Msg::AnthropicRaw { messages: vec![json!({ "role": "assistant", "content": o.raw_content })], fallback_text: o.text.clone() });
                }
                _ => break,
            }
        }
        if answer.trim().is_empty() {
            hub.publish(part("error", steps, "", Some("The sub-agent ended without a report.")));
            return ("The sub-agent ended without a report.".to_string(), true);
        }
        hub.publish(part("done", steps, &answer, None));
        (answer, false)
    })
}

/// Runs one of our tools; returns (content for the model, is_error). `agent`: the sub-agent
/// the call belongs to.
async fn run_tool(job: &Job, hub: &Hub, call: &ToolCall, t: &Tooling<'_>, agent: Option<&str>) -> (String, bool) {
    let (search_key, sources, cancel) = (t.search_key, t.sources, t.cancel);
    let ws = &job.settings.web_search;
    let started = Instant::now();
    let ours = call.name == WEB_SEARCH || call.name == FETCH_URL || sources.iter().any(|s| s.tool == call.name);
    if !ours && let Some(session) = &job.desktop {
        if call.name == TASK && agent.is_none() && session.policy.general.subagents {
            return run_agent(job, hub, call, t).await;
        }
        if let Some(tool) = session.find(&call.name) {
            return run_desktop_tool(job, hub, session, tool, call, cancel, agent, t.media).await;
        }
    }
    if let Some(src) = sources.iter().find(|s| s.tool == call.name) {
        let query = call.input["query"].as_str().unwrap_or_default().trim().to_string();
        let space = call.input["space"].as_str();
        if query.is_empty() {
            return ("The query is empty.".into(), true);
        }
        let name = src.connector.name.clone();
        let engine = format!("wikicraft:{name}");
        hub.publish(json!({ "type": "part", "part": { "type": "knowledge", "id": call.id, "source": name, "query": query, "status": "running" } }));
        let res = crate::connectors::search(src, &query, space).await;
        let ms = started.elapsed().as_millis() as i64;
        let _ = sqlx::query("UPDATE connector_tokens SET last_used_at = now() WHERE connector_id = $1 AND user_id = $2")
            .bind(src.connector.id)
            .bind(job.user.id)
            .execute(&job.state.db)
            .await;
        return match res {
            Ok(passages) => {
                let hits = crate::connectors::hits(&passages);
                hub.publish(json!({ "type": "part", "part": { "type": "knowledge", "id": call.id, "source": name, "query": query, "status": "done", "results": hits } }));
                record_web(job, "retrieve", &engine, Some(&query), None, &hits, None, ms).await;
                (crate::connectors::for_model(&name, &passages), false)
            }
            Err(e) => {
                hub.publish(
                    json!({ "type": "part", "part": { "type": "knowledge", "id": call.id, "source": name, "query": query, "status": "error", "error": e } }),
                );
                record_web(job, "retrieve", &engine, Some(&query), None, &[], Some(&e), ms).await;
                (format!("The search failed: {e}"), true)
            }
        };
    }
    match call.name.as_str() {
        WEB_SEARCH => {
            let query = call.input["query"].as_str().unwrap_or_default().trim().to_string();
            if query.is_empty() {
                return ("The query is empty.".into(), true);
            }
            hub.publish(json!({ "type": "part", "part": { "type": "search", "id": call.id, "engine": ws.engine, "query": query, "status": "running" } }));
            let res = crate::web::search(ws, search_key, &query).await;
            let ms = started.elapsed().as_millis() as i64;
            match res {
                Ok(hits) => {
                    hub.publish(json!({ "type": "part", "part": { "type": "search", "id": call.id, "engine": ws.engine, "query": query, "status": "done", "results": hits } }));
                    record_web(job, "search", &ws.engine, Some(&query), None, &hits, None, ms).await;
                    (crate::web::results_for_model(&hits), false)
                }
                Err(e) => {
                    hub.publish(json!({ "type": "part", "part": { "type": "search", "id": call.id, "engine": ws.engine, "query": query, "status": "error", "error": e } }));
                    record_web(job, "search", &ws.engine, Some(&query), None, &[], Some(&e), ms).await;
                    (format!("Search failed: {e}"), true)
                }
            }
        }
        FETCH_URL if ws.fetch_enabled => {
            let url = call.input["url"].as_str().unwrap_or_default().trim().to_string();
            hub.publish(json!({ "type": "part", "part": { "type": "fetch", "id": call.id, "url": url, "status": "running" } }));
            let res = crate::web::fetch(&url, ws, job.state.config.fetch_allow_private).await;
            let ms = started.elapsed().as_millis() as i64;
            match res {
                Ok(page) => {
                    hub.publish(json!({ "type": "part", "part": { "type": "fetch", "id": call.id, "url": page.url, "title": page.title, "status": "done" } }));
                    let hit = SearchHit { title: page.title.clone(), url: page.url.clone(), snippet: String::new() };
                    record_web(job, "fetch", "fetch", None, Some(&url), &[hit], None, ms).await;
                    let mut content = format!("Title: {}\nURL: {}\n\n{}", page.title, page.url, page.text);
                    if page.truncated {
                        content.push_str("\n\n[The page was longer; the rest was cut off.]");
                    }
                    (content, false)
                }
                Err(e) => {
                    hub.publish(json!({ "type": "part", "part": { "type": "fetch", "id": call.id, "url": url, "status": "error", "error": e } }));
                    record_web(job, "fetch", "fetch", None, Some(&url), &[], Some(&e), ms).await;
                    (format!("Couldn't open the page: {e}"), true)
                }
            }
        }
        other => (format!("Unknown tool {other}."), true),
    }
}

/// Runs a tool on the person's computer through their desktop app, shows it in the chat and
/// records it.
async fn run_desktop_tool(
    job: &Job,
    hub: &Hub,
    session: &crate::desktop::Session,
    tool: &crate::desktop::SessionTool,
    call: &ToolCall,
    cancel: &CancellationToken,
    agent: Option<&str>,
    returned: &Mutex<HashMap<usize, Vec<llm::Media>>>,
) -> (String, bool) {
    let started = Instant::now();
    // an id of our own: the model's ids repeat between answers and sub-agents
    let uid = new_id().to_string();
    let input = &crate::desktop::scrub(&call.input);
    let part = |status: &str, output: &str, error: Option<&str>| {
        json!({ "type": "part", "part": {
            "type": "tool", "id": uid, "agent": agent, "feature": tool.feature, "connector": tool.connector.as_ref().map(|c| c.1.as_str()),
            "tool": tool.remote_name, "input": input, "status": status, "output": crate::desktop::shown(output), "error": error } })
    };
    hub.publish(part("running", "", None));
    let mut so_far = String::new();
    let done = crate::desktop::call(&job.state, session, tool, &uid, job.chat_id, job.assistant_msg, input, cancel, |text| {
        so_far.push_str(text);
        // only the end is shown
        if so_far.len() > 32_000 {
            so_far = crate::desktop::shown(&so_far);
        }
        hub.publish(part("running", &so_far, None));
    })
    .await;
    let mut done = done;
    let files = std::mem::take(&mut done.media);
    let mut ended = match done.status {
        "ok" => part("done", &done.content, None),
        status => part(status, "", Some(&done.content)),
    };
    if !files.is_empty() {
        ended["part"]["media"] = json!(files.iter().map(|m| json!({ "name": m.name, "mediaType": m.media_type })).collect::<Vec<_>>());
        if job.model.cap("vision") {
            returned.lock().expect("media").insert(call as *const ToolCall as usize, files);
        } else {
            done.content.push_str("\n[The file itself wasn't passed on: this model can't read images or PDFs.]");
        }
    }
    hub.publish(ended);
    crate::desktop::record(&job.state, job.user.id, session, tool, job.chat_id, job.assistant_msg, input, &done, started).await;
    let full = job.settings.audit_content == "full";
    crate::audit::log(
        &job.state,
        Some(&job.user),
        Some(&job.client),
        "desktop.tool",
        Some(("chat", job.chat_id.to_string())),
        json!({
            "deviceId": session.device_id, "feature": tool.feature, "connector": tool.connector.as_ref().map(|c| c.1.as_str()), "tool": tool.remote_name,
            "input": full.then_some(input), "status": done.status, "resultChars": done.content.chars().count(), "messageId": job.assistant_msg,
            "durationMs": started.elapsed().as_millis() as u64,
        }),
    )
    .await;
    (done.content, done.is_error)
}

/// Stores a search or fetch and audits it.
async fn record_web(job: &Job, kind: &str, engine: &str, query: Option<&str>, url: Option<&str>, results: &[SearchHit], error: Option<&str>, ms: i64) {
    let status = match error {
        None => "ok",
        Some(e) if e.contains("blocked") || e.contains("private networks") => "blocked",
        Some(_) => "error",
    };
    let id = new_id();
    let res = sqlx::query(
        "INSERT INTO web_events (id, user_id, chat_id, message_id, kind, engine, query, url, results, result_count, status, error, duration_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(id)
    .bind(job.user.id)
    .bind(job.chat_id)
    .bind(job.assistant_msg)
    .bind(kind)
    .bind(engine)
    .bind(query)
    .bind(url)
    .bind(serde_json::to_value(results).unwrap_or(Value::Null))
    .bind(results.len() as i32)
    .bind(status)
    .bind(error)
    .bind(ms as i32)
    .execute(&job.state.db)
    .await;
    if let Err(e) = res {
        tracing::error!("web event record failed: {e}");
    }
    let urls: Vec<&str> = results.iter().map(|r| r.url.as_str()).collect();
    crate::audit::log(
        &job.state,
        Some(&job.user),
        Some(&job.client),
        match kind {
            "search" => "web.search",
            "fetch" => "web.fetch",
            _ => "knowledge.search",
        },
        Some(("chat", job.chat_id.to_string())),
        json!({ "engine": engine, "query": query, "url": url, "status": status, "error": error, "results": urls, "messageId": job.assistant_msg }),
    )
    .await;
}

/// Stores the answer, tells the browser, audits it and writes a title.
async fn finish(job: &Job, hub: &Hub, out: Outcome, started: Instant) {
    let state = &job.state;
    let (mut content, parts) = hub.snapshot();
    if let Some(note) = &out.note {
        let n = format!("\n\n*{note}*");
        hub.publish(json!({ "type": "text", "d": n }));
        content.push_str(&n);
    }
    let duration = started.elapsed().as_millis() as i32;
    let model_name = if out.model_name.is_empty() { job.model.model.clone() } else { out.model_name.clone() };
    let res = sqlx::query(
        "UPDATE messages SET content = $2, parts = $3, raw = $4, status = $5, error = $6, input_tokens = $7, output_tokens = $8, cost_usd = $9,
                duration_ms = $10, model_name = $11 WHERE id = $1",
    )
    .bind(job.assistant_msg)
    .bind(&content)
    .bind(Value::Array(parts))
    .bind(&out.raw)
    .bind(out.status)
    .bind(&out.error)
    .bind(out.usage.input + out.usage.cache_read + out.usage.cache_write)
    .bind(out.usage.output)
    .bind(out.cost)
    .bind(duration)
    .bind(&model_name)
    .execute(&state.db)
    .await;
    if let Err(e) = res {
        tracing::error!("storing the answer failed: {e}");
    }
    let _ = sqlx::query("UPDATE chats SET updated_at = now() WHERE id = $1").bind(job.chat_id).execute(&state.db).await;
    if let Ok(msg) = crate::routes::chats::load_message(state, job.assistant_msg).await {
        hub.publish(json!({ "type": "done", "message": msg }));
    }
    let full = job.settings.audit_content == "full";
    crate::audit::log(
        state,
        Some(&job.user),
        Some(&job.client),
        "chat.answer",
        Some(("chat", job.chat_id.to_string())),
        json!({
            "messageId": job.assistant_msg, "model": model_name, "modelId": job.model.id, "status": out.status, "error": out.error,
            "inputTokens": out.usage.input + out.usage.cache_read + out.usage.cache_write, "outputTokens": out.usage.output,
            "costUsd": out.cost, "chars": content.chars().count(), "content": full.then_some(&content),
        }),
    )
    .await;
    if job.title_after
        && out.status == "complete"
        && let Some(title) = write_title(job, &content).await
    {
        hub.publish(json!({ "type": "title", "title": title }));
    }
}

/// Asks a model for a short title of the chat (the title model, else the chat's own).
async fn write_title(job: &Job, answer: &str) -> Option<String> {
    let state = &job.state;
    let prompt: String = sqlx::query_scalar("SELECT content FROM messages WHERE id = $1").bind(job.user_msg).fetch_one(&state.db).await.ok()?;
    let fallback: String = {
        let first = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("New chat").trim();
        let t: String = first.chars().take(60).collect();
        if first.chars().count() > 60 { format!("{t}…") } else { t }
    };
    let model = match job.settings.title_model_id {
        Some(id) => crate::catalog::for_user(state, &job.user, id).await.unwrap_or_else(|_| job.model.clone()),
        None => job.model.clone(),
    };
    let ep = crate::catalog::endpoint(state, model.provider_id).await.ok()?;
    let excerpt = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    let req = llm::Request {
        model: model.model.clone(),
        system: Some("You name conversations. Reply with a title of at most six words in the language of the conversation: no quotes, no punctuation at the end, nothing else.".into()),
        messages: vec![Msg::user(format!("<user>{}</user>\n<assistant>{}</assistant>\n\nTitle:", excerpt(&prompt, 2000), excerpt(answer, 2000)))],
        effort: (model.kind() == Kind::Anthropic && model.cap("reasoning")).then(|| "low".into()),
        max_tokens: Some(2000),
        ..Default::default()
    };
    let started = Instant::now();
    let res = llm::stream(&llm::client(60), &ep, &req, &CancellationToken::new(), &mut |_| {}).await;
    let title = match res {
        Ok(o) => {
            crate::usage::record(
                state,
                crate::usage::Record {
                    user: Some(job.user.id),
                    chat: Some(job.chat_id),
                    message: None,
                    model: &model,
                    source: "title",
                    usage: o.usage,
                    latency_ms: started.elapsed().as_millis() as i64,
                    status: "ok",
                    error: None,
                },
            )
            .await;
            let t = o.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").trim_matches(['"', '\'', '*', '#', '.', ' ']).to_string();
            let t = t.strip_prefix("Title:").map(str::trim).unwrap_or(&t).to_string();
            if t.is_empty() { fallback } else { t.chars().take(80).collect() }
        }
        Err(e) => {
            tracing::debug!("title generation failed: {e}");
            fallback
        }
    };
    sqlx::query("UPDATE chats SET title = $2 WHERE id = $1 AND title = 'New chat'").bind(job.chat_id).bind(&title).execute(&state.db).await.ok()?;
    Some(title)
}

/// Answers left streaming by a stopped server are marked as interrupted.
pub async fn recover(state: &AppState) -> anyhow::Result<()> {
    let n = sqlx::query("UPDATE messages SET status = 'error', error = 'interrupted: the server restarted while answering' WHERE status = 'streaming'")
        .execute(&state.db)
        .await?
        .rows_affected();
    if n > 0 {
        tracing::warn!("{n} answers were interrupted by a restart");
    }
    Ok(())
}
