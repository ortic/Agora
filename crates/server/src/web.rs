//! Web access for models: search (SearXNG, Brave Search API, Tavily) and page fetches.
//! Fetches refuse private, loopback and link-local addresses (the resolved address is checked
//! and pinned, so DNS rebinding can't sneak past; every redirect is checked again).

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use serde_json::{Value, json};

use crate::llm::SearchHit;
use crate::settings::WebSearch;

const MAX_PAGE_BYTES: usize = 3 * 1024 * 1024;
/// characters of page text handed to the model
pub const MAX_PAGE_CHARS: usize = 40_000;

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Mozilla/5.0 (compatible; Agora/0.1; +https://github.com/)")
        .build()
        .expect("http client")
}

pub async fn search(ws: &WebSearch, api_key: Option<&str>, query: &str) -> Result<Vec<SearchHit>, String> {
    let n = ws.max_results.clamp(1, 20) as usize;
    let client = http();
    let hits: Vec<SearchHit> = match ws.engine.as_str() {
        "searxng" => {
            let base = ws.url.as_deref().filter(|u| !u.trim().is_empty()).ok_or("SearXNG needs a URL")?;
            let r = client
                .get(format!("{}/search", base.trim_end_matches('/')))
                .query(&[("q", query), ("format", "json")])
                .send()
                .await
                .map_err(|e| format!("SearXNG can't be reached: {e}"))?;
            let v = json_or_error(r, "SearXNG").await?;
            v["results"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| SearchHit {
                    title: r["title"].as_str().unwrap_or_default().into(),
                    url: r["url"].as_str().unwrap_or_default().into(),
                    snippet: r["content"].as_str().unwrap_or_default().into(),
                })
                .collect()
        }
        "brave" => {
            let key = api_key.ok_or("Brave Search needs an API key")?;
            let r = client
                .get("https://api.search.brave.com/res/v1/web/search")
                .query(&[("q", query), ("count", &n.to_string())])
                .header("X-Subscription-Token", key)
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| format!("Brave Search can't be reached: {e}"))?;
            let v = json_or_error(r, "Brave Search").await?;
            v["web"]["results"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| SearchHit {
                    title: r["title"].as_str().unwrap_or_default().into(),
                    url: r["url"].as_str().unwrap_or_default().into(),
                    snippet: strip_tags(r["description"].as_str().unwrap_or_default()),
                })
                .collect()
        }
        "serper" => {
            // Google results through serper.dev
            let key = api_key.ok_or("Serper needs an API key")?;
            let r = client
                .post("https://google.serper.dev/search")
                .header("X-API-KEY", key)
                .json(&json!({ "q": query, "num": n }))
                .send()
                .await
                .map_err(|e| format!("Serper can't be reached: {e}"))?;
            let v = json_or_error(r, "Serper").await?;
            let mut hits: Vec<SearchHit> = Vec::new();
            // a direct answer, when Google has one, first
            if let Some(link) = v["answerBox"]["link"].as_str() {
                let snippet = v["answerBox"]["answer"].as_str().or(v["answerBox"]["snippet"].as_str()).unwrap_or_default();
                hits.push(SearchHit { title: v["answerBox"]["title"].as_str().unwrap_or(link).into(), url: link.into(), snippet: snippet.into() });
            }
            hits.extend(v["organic"].as_array().into_iter().flatten().map(|r| SearchHit {
                title: r["title"].as_str().unwrap_or_default().into(),
                url: r["link"].as_str().unwrap_or_default().into(),
                snippet: r["snippet"].as_str().unwrap_or_default().into(),
            }));
            hits.dedup_by(|a, b| a.url == b.url);
            hits
        }
        "tavily" => {
            let key = api_key.ok_or("Tavily needs an API key")?;
            let r = client
                .post("https://api.tavily.com/search")
                .bearer_auth(key)
                .json(&json!({ "query": query, "max_results": n }))
                .send()
                .await
                .map_err(|e| format!("Tavily can't be reached: {e}"))?;
            let v = json_or_error(r, "Tavily").await?;
            v["results"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| SearchHit {
                    title: r["title"].as_str().unwrap_or_default().into(),
                    url: r["url"].as_str().unwrap_or_default().into(),
                    snippet: r["content"].as_str().unwrap_or_default().into(),
                })
                .collect()
        }
        other => return Err(format!("web search engine {other:?} isn't set up")),
    };
    Ok(hits
        .into_iter()
        .filter(|h| h.url.starts_with("http"))
        .filter(|h| url::Url::parse(&h.url).ok().and_then(|u| u.host_str().map(|h| !ws.blocked(h))).unwrap_or(false))
        .map(|mut h| {
            h.snippet = h.snippet.chars().take(500).collect();
            h
        })
        .take(n)
        .collect())
}

async fn json_or_error(r: reqwest::Response, what: &str) -> Result<Value, String> {
    let status = r.status();
    let text = r.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("{what} answered HTTP {}: {}", status.as_u16(), text.chars().take(200).collect::<String>()));
    }
    serde_json::from_str(&text).map_err(|_| format!("{what} didn't answer with JSON (for SearXNG: enable the json format in settings.yml)"))
}

/// Whether an address is one a fetch must not reach.
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64 // carrier-grade NAT
                || v4.octets()[0] == 0
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private(IpAddr::V4(v4));
            }
            v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

pub struct Page {
    pub url: String,
    pub title: String,
    pub text: String,
    pub truncated: bool,
}

/// Fetches a page and returns its readable text.
pub async fn fetch(raw_url: &str, ws: &WebSearch, allow_private: bool) -> Result<Page, String> {
    let mut url = url::Url::parse(raw_url.trim()).map_err(|_| "that isn't a valid URL".to_string())?;
    for _hop in 0..5 {
        if !matches!(url.scheme(), "http" | "https") {
            return Err("only http and https addresses can be opened".into());
        }
        let host = url.host_str().ok_or("the URL has no host")?.to_string();
        if ws.blocked(&host) {
            return Err(format!("{host} is blocked by the administrator"));
        }
        let port = url.port_or_known_default().unwrap_or(443);
        let host_for_dns = host.trim_start_matches('[').trim_end_matches(']').to_string();
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host_for_dns.as_str(), port)).await.map_err(|_| format!("{host} can't be found"))?.collect();
        let Some(addr) = addrs.first().copied() else { return Err(format!("{host} can't be found")) };
        if !allow_private && addrs.iter().any(|a| is_private(a.ip())) {
            return Err("addresses in private networks can't be opened".into());
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .resolve(&host_for_dns, addr)
            .user_agent("Mozilla/5.0 (compatible; Agora/0.1)")
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client
            .get(url.clone())
            .header("Accept", "text/html,application/xhtml+xml,text/plain,application/json;q=0.9,*/*;q=0.5")
            .send()
            .await
            .map_err(|e| format!("the page can't be loaded: {e}"))?;
        if resp.status().is_redirection() {
            let loc = resp.headers().get("location").and_then(|l| l.to_str().ok()).ok_or("redirect without a location")?;
            url = url.join(loc).map_err(|_| "invalid redirect".to_string())?;
            continue;
        }
        if !resp.status().is_success() {
            return Err(format!("the page answered HTTP {}", resp.status().as_u16()));
        }
        let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
        if !(ctype.is_empty() || ctype.contains("html") || ctype.starts_with("text/") || ctype.contains("json") || ctype.contains("xml")) {
            return Err(format!("this kind of file can't be read ({})", ctype.split(';').next().unwrap_or("")));
        }
        let mut body: Vec<u8> = Vec::new();
        let mut resp = resp;
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_PAGE_BYTES {
                break;
            }
        }
        let raw = String::from_utf8_lossy(&body).into_owned();
        let (title, text) =
            if ctype.contains("html") || (ctype.is_empty() && raw.trim_start().starts_with('<')) { html_to_text(&raw) } else { (String::new(), raw) };
        let truncated = text.chars().count() > MAX_PAGE_CHARS;
        let text: String = text.chars().take(MAX_PAGE_CHARS).collect();
        return Ok(Page { url: url.to_string(), title, text, truncated });
    }
    Err("too many redirects".into())
}

fn strip_tags(s: &str) -> String {
    html_to_text(s).1
}

/// Readable text of an HTML page: no scripts, styles or markup; block elements on lines of
/// their own. Returns (title, text).
pub fn html_to_text(html: &str) -> (String, String) {
    let mut out = String::with_capacity(html.len() / 3);
    let mut title = String::new();
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut i = 0;
    let mut in_title = false;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // comments
            if lower[i..].starts_with("<!--") {
                i = lower[i..].find("-->").map(|e| i + e + 3).unwrap_or(bytes.len());
                continue;
            }
            let end = match lower[i..].find('>') {
                Some(e) => i + e,
                None => break,
            };
            let tag = &lower[i + 1..end];
            let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            let closing = tag.starts_with('/');
            if !closing && matches!(name.as_str(), "script" | "style" | "noscript" | "svg" | "template" | "iframe") {
                let close = format!("</{name}");
                i = lower[end..].find(&close).map(|e| end + e).and_then(|s| lower[s..].find('>').map(|g| s + g + 1)).unwrap_or(bytes.len());
                continue;
            }
            if name == "title" {
                in_title = !closing;
            }
            if matches!(
                name.as_str(),
                "br" | "p"
                    | "div"
                    | "li"
                    | "tr"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "section"
                    | "article"
                    | "header"
                    | "footer"
                    | "pre"
                    | "blockquote"
                    | "table"
                    | "ul"
                    | "ol"
                    | "nav"
            ) {
                out.push('\n');
                if !closing && name == "li" {
                    out.push_str("- ");
                }
                if !closing && name.len() == 2 && name.starts_with('h') {
                    out.push_str(&"#".repeat(name[1..].parse::<usize>().unwrap_or(1)));
                    out.push(' ');
                }
            } else if matches!(name.as_str(), "td" | "th") {
                out.push_str(" | ");
            }
            i = end + 1;
            continue;
        }
        let next = lower[i..].find('<').map(|n| i + n).unwrap_or(bytes.len());
        let chunk = decode_entities(&html[i..next]);
        if in_title {
            title.push_str(&chunk);
        } else {
            out.push_str(&chunk);
        }
        i = next;
    }
    // collapse whitespace: single spaces within lines, at most one blank line
    let mut text = String::with_capacity(out.len());
    let mut blank = 0;
    for line in out.lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() || l == "|" || l == "-" {
            blank += 1;
            if blank == 1 && !text.is_empty() {
                text.push('\n');
            }
            continue;
        }
        blank = 0;
        text.push_str(&l);
        text.push('\n');
    }
    (title.split_whitespace().collect::<Vec<_>>().join(" "), text.trim().to_string())
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let Some(semi) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// What a model gets back from a search.
pub fn results_for_model(hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return "No results.".into();
    }
    hits.iter().enumerate().map(|(i, h)| format!("[{}] {}\n{}\n{}", i + 1, h.title, h.url, h.snippet)).collect::<Vec<_>>().join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_becomes_readable_text() {
        let (title, text) = html_to_text(
            "<html><head><title>My &amp; Page</title><style>p{x}</style></head><body><script>alert(1)</script>\
             <h2>Intro</h2><p>Hello&nbsp;<b>world</b> &#8211; ok</p><!-- hidden --><ul><li>one</li><li>two</li></ul></body></html>",
        );
        assert_eq!(title, "My & Page");
        assert!(text.contains("## Intro"), "{text}");
        assert!(text.contains("Hello world – ok"), "{text}");
        assert!(text.contains("- one\n- two") || text.contains("- one\n\n- two"), "{text}");
        assert!(!text.contains("alert") && !text.contains("p{x}") && !text.contains("hidden"));
    }

    #[test]
    fn private_addresses() {
        for a in ["127.0.0.1", "10.1.2.3", "192.168.0.1", "169.254.169.254", "::1", "fd00::1", "::ffff:10.0.0.1", "100.64.0.1", "0.0.0.0"] {
            assert!(is_private(a.parse().unwrap()), "{a}");
        }
        for a in ["8.8.8.8", "2606:4700::1111", "1.1.1.1"] {
            assert!(!is_private(a.parse().unwrap()), "{a}");
        }
    }

    #[tokio::test]
    async fn fetch_refuses_private_hosts() {
        let ws = WebSearch::default();
        let e = fetch("http://127.0.0.1:1/", &ws, false).await.err().unwrap();
        assert!(e.contains("private"), "{e}");
        assert!(fetch("file:///etc/passwd", &ws, false).await.is_err());
        let blocked = WebSearch { blocked_domains: vec!["localhost".into()], ..Default::default() };
        assert!(fetch("http://localhost/", &blocked, true).await.err().unwrap().contains("blocked"));
    }
}
