//! Known Claude models with their list prices (USD per million tokens), so discovered models
//! come with sensible defaults. Administrators can change every value; prices of other
//! providers are entered by hand (they differ per contract and change often).

pub struct Preset {
    pub model: &'static str,
    pub name: &'static str,
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub context: i32,
    pub max_output: i32,
}

pub const CLAUDE: &[Preset] = &[
    Preset { model: "claude-fable-5-2", name: "Claude Fable 5.2", input: 8.0, output: 40.0, cache_read: 0.20, context: 1_000_000, max_output: 128_000 },
    Preset { model: "claude-opus-5-5", name: "Claude Opus 5.5", input: 4.0, output: 20.0, cache_read: 0.20, context: 1_000_000, max_output: 128_000 },
    Preset { model: "claude-sonnet-5-5", name: "Claude Sonnet 5.5", input: 2.0, output: 10.0, cache_read: 0.10, context: 1_000_000, max_output: 128_000 },
    Preset { model: "claude-haiku-5-5", name: "Claude Haiku 5.5", input: 0.10, output: 0.50, cache_read: 0.01, context: 1_000_000, max_output: 128_000 },
    Preset { model: "claude-fable-5-1", name: "Claude Fable 5.1", input: 10.0, output: 50.0, cache_read: 1.0, context: 1_000_000, max_output: 128_000 },
    Preset { model: "claude-fable-5", name: "Claude Fable 5", input: 10.0, output: 50.0, cache_read: 1.0, context: 1_000_000, max_output: 128_000 },
    Preset { model: "claude-opus-5", name: "Claude Opus 5", input: 5.0, output: 25.0, cache_read: 0.50, context: 1_000_000, max_output: 128_000 },
    Preset { model: "claude-sonnet-5", name: "Claude Sonnet 5", input: 2.0, output: 10.0, cache_read: 0.20, context: 1_000_000, max_output: 128_000 },
    Preset { model: "claude-opus-4-8", name: "Claude Opus 4.8", input: 5.0, output: 25.0, cache_read: 0.50, context: 1_000_000, max_output: 128_000 },
    Preset { model: "claude-sonnet-4-6", name: "Claude Sonnet 4.6", input: 3.0, output: 15.0, cache_read: 0.30, context: 1_000_000, max_output: 128_000 },
    Preset { model: "claude-haiku-4-5", name: "Claude Haiku 4.5", input: 1.0, output: 5.0, cache_read: 0.10, context: 200_000, max_output: 64_000 },
];

/// Claude's server-side web search: $10 per 1,000 searches.
pub const CLAUDE_SEARCH_PRICE: f64 = 0.01;

pub fn claude(model: &str) -> Option<&'static Preset> {
    CLAUDE.iter().find(|p| p.model == model)
}

/// 5-minute cache writes cost 1.25 × the input price.
pub fn cache_write_price(input: f64) -> f64 {
    (input * 1.25 * 1000.0).round() / 1000.0
}

/// Claude models of the 4.6 generation and later think adaptively; older ones don't take it.
pub fn claude_thinks(model: &str) -> bool {
    !(model.contains("haiku-4") || model.contains("-3-") || model.starts_with("claude-3") || model.contains("4-5") || model.contains("4-1") || model.contains("-4-0") || model.ends_with("-4"))
}
