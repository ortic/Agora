//! Minimal Server-Sent Events reader over a reqwest byte stream.

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::LlmError;

pub struct SseEvent {
    /// the `event:` field (Anthropic names its events; the type is in the data too)
    #[allow(dead_code)]
    pub event: String,
    pub data: String,
}

pub struct Reader {
    body: futures::stream::BoxStream<'static, reqwest::Result<bytes::Bytes>>,
    buf: Vec<u8>,
    timeout_secs: u64,
}

impl Reader {
    pub fn new(resp: reqwest::Response, timeout_secs: u64) -> Self {
        Reader { body: resp.bytes_stream().boxed(), buf: Vec::new(), timeout_secs }
    }

    /// The next event; `Ok(None)` at the end of the stream.
    pub async fn next(&mut self, cancel: &CancellationToken) -> Result<Option<SseEvent>, LlmError> {
        loop {
            if let Some(ev) = self.take_event() {
                return Ok(Some(ev));
            }
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(LlmError::Http("stopped".into())),
                c = self.body.next() => c,
            };
            match chunk {
                Some(Ok(b)) => self.buf.extend_from_slice(&b),
                Some(Err(e)) if e.is_timeout() => return Err(LlmError::Timeout(self.timeout_secs)),
                Some(Err(e)) => return Err(LlmError::Http(format!("the model's answer broke off: {e}"))),
                None => {
                    // a last event without the blank line after it
                    if !self.buf.iter().all(|b| b.is_ascii_whitespace()) {
                        self.buf.extend_from_slice(b"\n\n");
                        continue;
                    }
                    return Ok(None);
                }
            }
        }
    }

    fn take_event(&mut self) -> Option<SseEvent> {
        loop {
            // events end with a blank line (\n\n or \r\n\r\n)
            let end = find_blank_line(&self.buf)?;
            let raw: Vec<u8> = self.buf.drain(..end.1).collect();
            let text = String::from_utf8_lossy(&raw[..end.0]);
            let mut event = String::new();
            let mut data: Vec<&str> = Vec::new();
            for line in text.lines() {
                let line = line.trim_end_matches('\r');
                if let Some(v) = line.strip_prefix("event:") {
                    event = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("data:") {
                    data.push(v.strip_prefix(' ').unwrap_or(v));
                }
            }
            if !data.is_empty() || !event.is_empty() {
                return Some(SseEvent { event, data: data.join("\n") });
            }
        }
    }
}

/// (end of the event's text, end including the blank line)
fn find_blank_line(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i + 1 < buf.len() {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some((i, i + 2));
        }
        if i + 3 < buf.len() && &buf[i..i + 4] == b"\r\n\r\n" {
            return Some((i, i + 4));
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn blank_lines() {
        assert_eq!(super::find_blank_line(b"data: a\n\ndata: b"), Some((7, 9)));
        assert_eq!(super::find_blank_line(b"data: a\r\n\r\n"), Some((7, 11)));
        assert_eq!(super::find_blank_line(b"data: a\n"), None);
    }
}
