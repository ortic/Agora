//! Talking to models: one provider-neutral request and streaming interface, with adapters for
//! the Anthropic Messages API (Claude) and the OpenAI-compatible chat completions API (OpenAI,
//! gateways such as LiteLLM or OpenRouter, and models in your own network: Ollama, vLLM,
//! llama.cpp, LM Studio, LocalAI, …).

pub mod anthropic;
pub mod openai;
pub mod presets;
mod sse;

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// How to reach a provider.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub kind: Kind,
    pub base_url: String,
    pub api_key: Option<String>,
    pub headers: BTreeMap<String, String>,
    /// seconds to wait for the first byte and between streamed chunks
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Anthropic,
    Openai,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "anthropic" => Some(Kind::Anthropic),
            "openai" => Some(Kind::Openai),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub enum Block {
    Text(String),
    /// base64 image
    Image {
        media_type: String,
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// a plain turn
    Plain { role: Role, content: Vec<Block> },
    /// messages exactly as an Anthropic model returned them (thinking signatures, server tool
    /// results): replayed unchanged to Claude models, reduced to `fallback_text` for others
    AnthropicRaw { messages: Vec<Value>, fallback_text: String },
}

impl Msg {
    pub fn user(text: impl Into<String>) -> Msg {
        Msg::Plain { role: Role::User, content: vec![Block::Text(text.into())] }
    }
    pub fn assistant(text: impl Into<String>) -> Msg {
        Msg::Plain { role: Role::Assistant, content: vec![Block::Text(text.into())] }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

/// Claude's own (server-side) web search.
#[derive(Debug, Clone)]
pub struct NativeSearch {
    pub max_uses: u32,
    pub blocked_domains: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Request {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Msg>,
    pub tools: Vec<ToolDef>,
    pub native_search: Option<NativeSearch>,
    /// Claude: `output_config.effort`; OpenAI-compatible: `reasoning_effort`
    pub effort: Option<String>,
    /// OpenAI-compatible only (the current Anthropic API takes no sampling parameters)
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    /// Claude: ask for adaptive thinking with summaries shown
    pub thinking: bool,
    /// merged into the JSON body (OpenAI-compatible servers with extra options)
    pub extra_body: Option<Value>,
}

/// What streams to the user while the model writes.
#[derive(Debug, Clone)]
pub enum Event {
    Text(String),
    Reasoning(String),
    /// the model started calling a tool
    ToolStart {
        id: String,
        name: String,
    },
    /// Claude searched the web itself
    NativeSearch {
        id: String,
        query: String,
    },
    NativeSearchResults {
        id: String,
        results: Vec<SearchHit>,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub snippet: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// input tokens not read from a cache
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    /// searches the provider ran itself
    pub searches: i64,
}

impl Usage {
    pub fn add(&mut self, o: &Usage) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.searches += o.searches;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    End,
    ToolUse,
    MaxTokens,
    Refusal,
    /// Claude paused a long server-side tool turn; send the turn back to continue
    Pause,
    Cancelled,
    Other(String),
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// One model call's result.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop: Stop,
    pub usage: Usage,
    /// the model that answered, as the provider reports it
    pub model: String,
    /// Anthropic: the assistant content blocks as returned (to send back unchanged)
    pub raw_content: Option<Vec<Value>>,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("{0}")]
    Http(String),
    /// the provider answered with an error status
    #[error("{message}")]
    Status { status: u16, message: String },
    #[error("no answer from the model within {0} seconds")]
    Timeout(u64),
}

pub fn client(timeout_secs: u64) -> reqwest::Client {
    reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).read_timeout(Duration::from_secs(timeout_secs.max(10))).build().expect("http client")
}

/// Streams one model call. `on_event` sees text, reasoning and tool activity as it arrives.
pub async fn stream(
    http: &reqwest::Client,
    ep: &Endpoint,
    req: &Request,
    cancel: &CancellationToken,
    on_event: &mut (dyn FnMut(Event) + Send),
) -> Result<Outcome, LlmError> {
    match ep.kind {
        Kind::Anthropic => anthropic::stream(http, ep, req, cancel, on_event).await,
        Kind::Openai => openai::stream(http, ep, req, cancel, on_event).await,
    }
}

/// A model a provider offers (discovery).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovered {
    pub model: String,
    pub display_name: String,
    pub context_window: Option<i64>,
    pub max_output_tokens: Option<i64>,
}

pub async fn list_models(http: &reqwest::Client, ep: &Endpoint) -> Result<Vec<Discovered>, LlmError> {
    match ep.kind {
        Kind::Anthropic => anthropic::list_models(http, ep).await,
        Kind::Openai => openai::list_models(http, ep).await,
    }
}

/// Sends a request, retrying while the service is busy (429, 5xx, 529) or unreachable.
pub(crate) async fn send(build: impl Fn() -> reqwest::RequestBuilder, cancel: &CancellationToken, timeout_secs: u64) -> Result<reqwest::Response, LlmError> {
    const ATTEMPTS: u64 = 3;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let res = tokio::select! {
            _ = cancel.cancelled() => return Err(LlmError::Http("stopped".into())),
            r = build().send() => r,
        };
        let r = match res {
            Ok(r) => r,
            Err(e) if e.is_timeout() => return Err(LlmError::Timeout(timeout_secs)),
            Err(e) if attempt < ATTEMPTS && e.is_connect() => {
                tracing::warn!("model request failed ({e}), retrying");
                tokio::time::sleep(Duration::from_millis(800 * attempt)).await;
                continue;
            }
            Err(e) => return Err(LlmError::Http(format!("the model service can't be reached: {}", error_chain(&e)))),
        };
        let status = r.status().as_u16();
        if (status == 429 || status >= 500) && attempt < ATTEMPTS {
            let wait = r.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(attempt).min(10);
            tracing::warn!(status, "model service busy, retrying in {wait}s");
            tokio::time::sleep(Duration::from_secs(wait)).await;
            continue;
        }
        if !(200..300).contains(&status) {
            let bytes = r.bytes().await.unwrap_or_default();
            let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let msg = v["error"]["message"]
                .as_str()
                .or(v["error"].as_str())
                .or(v["message"].as_str())
                .or(v["detail"].as_str())
                .map(String::from)
                .unwrap_or_else(|| String::from_utf8_lossy(&bytes).chars().take(300).collect());
            let hint = match status {
                401 | 403 => " (check the provider's API key)",
                404 => " (check the model name and the provider's base URL)",
                429 => " (rate limit at the provider)",
                _ => "",
            };
            return Err(LlmError::Status { status, message: format!("{msg}{hint}") });
        }
        return Ok(r);
    }
}

fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(inner) = src {
        s.push_str(": ");
        s.push_str(&inner.to_string());
        src = inner.source();
    }
    s
}

/// Splits `<think>…</think>` (reasoning some local models write into the answer) from the
/// answer while streaming. Tags may arrive split across chunks.
#[derive(Default)]
pub struct ThinkSplitter {
    in_think: bool,
    pending: String,
}

impl ThinkSplitter {
    /// Returns (answer text, reasoning text) for this chunk.
    pub fn push(&mut self, chunk: &str) -> (String, String) {
        self.pending.push_str(chunk);
        let (mut text, mut reasoning) = (String::new(), String::new());
        loop {
            let tag = if self.in_think { "</think>" } else { "<think>" };
            if let Some(pos) = self.pending.find(tag) {
                let before: String = self.pending[..pos].to_string();
                if self.in_think {
                    reasoning.push_str(&before);
                } else {
                    text.push_str(&before);
                }
                self.pending.drain(..pos + tag.len());
                self.in_think = !self.in_think;
                continue;
            }
            // keep a possible partial tag at the end for the next chunk
            let keep = (1..tag.len()).rev().find(|&n| self.pending.ends_with(&tag[..n])).unwrap_or(0);
            let cut = self.pending.len() - keep;
            let out: String = self.pending[..cut].to_string();
            self.pending.drain(..cut);
            if self.in_think {
                reasoning.push_str(&out);
            } else {
                text.push_str(&out);
            }
            break;
        }
        (text, reasoning)
    }

    pub fn finish(&mut self) -> (String, String) {
        let rest = std::mem::take(&mut self.pending);
        if self.in_think { (String::new(), rest) } else { (rest, String::new()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunks: &[&str]) -> (String, String) {
        let mut s = ThinkSplitter::default();
        let (mut t, mut r) = (String::new(), String::new());
        for c in chunks {
            let (a, b) = s.push(c);
            t += &a;
            r += &b;
        }
        let (a, b) = s.finish();
        (t + &a, r + &b)
    }

    #[test]
    fn think_tags_are_split_out_even_across_chunks() {
        assert_eq!(run(&["<think>hmm</think>Answer"]), ("Answer".into(), "hmm".into()));
        assert_eq!(run(&["<thi", "nk>a", "b</th", "ink>X", "Y"]), ("XY".into(), "ab".into()));
        assert_eq!(run(&["plain <", "b>bold</b>"]), ("plain <b>bold</b>".into(), String::new()));
        assert_eq!(run(&["<think>never closed"]), (String::new(), "never closed".into()));
    }
}
