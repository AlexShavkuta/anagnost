//! Minimal Claude Messages API client over raw HTTP (there is no official Rust SDK).

use anyhow::{Result, anyhow, bail};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2026-10-21";
/// Lets the API re-run a request on another model when a safety classifier declines it.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// Building the map is many short, well-defined summaries: the fast, low-cost model does it well.
pub const DEFAULT_MAP_MODEL: &str = "claude-haiku-5-5";

/// Tokens used, as reported by the API (thinking counts as output).
#[derive(Serialize, Clone, Copy, Default, Debug, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Usage {
    fn add_from(&mut self, u: &Value) {
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        self.input += n("input_tokens");
        self.output += n("output_tokens");
        self.cache_read += n("cache_read_input_tokens");
        self.cache_write += n("cache_creation_input_tokens");
    }

    /// Estimated cost in US dollars at list prices, if the model's price is known.
    pub fn cost(&self, model: &str) -> Option<f64> {
        let (inp, out, read) = price(model)?;
        let m = 1_000_000.0;
        // 5-minute cache writes cost 1.25x the input price.
        Some((self.input as f64 * inp + self.output as f64 * out + self.cache_read as f64 * read + self.cache_write as f64 * inp * 1.25) / m)
    }
}

/// List prices in US$ per million tokens: (input, output, cache read). Haiku 5.5 is priced for
/// prompts up to 100K tokens; map requests stay far below that.
pub fn price(model: &str) -> Option<(f64, f64, f64)> {
    let m = model.trim();
    Some(match m {
        _ if m.starts_with("claude-haiku-5-5") => (0.10, 0.50, 0.01),
        _ if m.starts_with("claude-sonnet-5-5") => (2.0, 10.0, 0.10),
        _ if m.starts_with("claude-opus-5-5") => (4.0, 20.0, 0.20),
        _ if m.starts_with("claude-fable-5-2") => (8.0, 40.0, 0.20),
        _ => return None,
    })
}

/// Shared tally of tokens used by a run of requests (e.g. one map build).
pub type Meter = Arc<Mutex<Usage>>;

#[derive(Clone)]
pub struct Claude {
    http: reqwest::Client,
    api_key: String,
    model: String,
    meter: Option<Meter>,
}

pub struct Request {
    /// System prompt blocks. The last one gets a cache breakpoint, so put large stable
    /// content (the section text) there and keep it identical across turns.
    pub system: Vec<String>,
    pub messages: Vec<Value>,
    pub max_tokens: u32,
    pub effort: &'static str,
    /// JSON schema for structured output, if any.
    pub schema: Option<Value>,
}

impl Claude {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            model: model.filter(|m| !m.is_empty()).unwrap_or_else(|| DEFAULT_MODEL.into()),
            meter: None,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Count this client's token use into `meter`.
    pub fn with_meter(mut self, meter: Meter) -> Self {
        self.meter = Some(meter);
        self
    }

    fn record(&self, usage: &Value) {
        if let Some(m) = &self.meter {
            m.lock().unwrap().add_from(usage);
        }
    }

    /// Send, retrying rate limits (429) and overload (529, 503) with backoff.
    async fn send(&self, body: &Value) -> Result<reqwest::Response> {
        let mut delay = Duration::from_secs(2);
        for attempt in 0..5 {
            let resp = self.post(body).send().await?;
            let code = resp.status().as_u16();
            if !matches!(code, 429 | 503 | 529) || attempt == 4 {
                return Ok(resp);
            }
            let wait = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs)
                .unwrap_or(delay)
                .min(Duration::from_secs(60));
            tokio::time::sleep(wait).await;
            delay *= 2;
        }
        unreachable!()
    }

    fn body(&self, req: &Request, stream: bool) -> Value {
        let n = req.system.len();
        let system: Vec<Value> = req
            .system
            .iter()
            .enumerate()
            .map(|(i, text)| {
                if i + 1 == n {
                    json!({"type": "text", "text": text, "cache_control": {"type": "ephemeral"}})
                } else {
                    json!({"type": "text", "text": text})
                }
            })
            .collect();
        let mut output_config = json!({"effort": req.effort});
        if let Some(schema) = &req.schema {
            output_config["format"] = json!({"type": "json_schema", "schema": schema});
        }
        json!({
            "model": self.model,
            "max_tokens": req.max_tokens,
            "system": system,
            "messages": req.messages,
            "output_config": output_config,
            "fallbacks": "default",
            "stream": stream,
        })
    }

    fn post(&self, body: &Value) -> reqwest::RequestBuilder {
        self.http
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("anthropic-beta", FALLBACK_BETA)
            .header("content-type", "application/json")
            .json(body)
    }

    /// Non-streaming call; returns the concatenated text blocks.
    pub async fn complete(&self, req: &Request) -> Result<String> {
        let resp = self.send(&self.body(req, false)).await?;
        let status = resp.status();
        let v: Value = resp.json().await?;
        if !status.is_success() {
            bail!(api_error(status.as_u16(), &v));
        }
        self.record(&v["usage"]);
        check_stop(&v["stop_reason"], &v["stop_details"])?;
        let text: String = v["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect();
        Ok(text)
    }

    /// Streaming call; `on_text` receives each text delta. Returns the full text.
    pub async fn stream(&self, req: &Request, mut on_text: impl FnMut(&str)) -> Result<String> {
        let resp = self.send(&self.body(req, true)).await?;
        let status = resp.status();
        if !status.is_success() {
            let v: Value = resp.json().await.unwrap_or(Value::Null);
            bail!(api_error(status.as_u16(), &v));
        }
        let mut full = String::new();
        let mut buf = String::new();
        let mut bytes = resp.bytes_stream();
        while let Some(chunk) = bytes.next().await {
            buf.push_str(&String::from_utf8_lossy(&chunk?));
            // SSE events are separated by a blank line.
            while let Some(end) = buf.find("\n\n") {
                let event: String = buf.drain(..end + 2).collect();
                for line in event.lines() {
                    let Some(data) = line.strip_prefix("data: ") else { continue };
                    let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
                    match v["type"].as_str() {
                        Some("message_start") => self.record(&v["message"]["usage"]),
                        Some("content_block_delta") if v["delta"]["type"] == "text_delta" => {
                            if let Some(t) = v["delta"]["text"].as_str() {
                                full.push_str(t);
                                on_text(t);
                            }
                        }
                        Some("message_delta") => {
                            // Output tokens arrive here; input was counted at message_start.
                            if let Some(out) = v["usage"]["output_tokens"].as_u64() {
                                self.record(&json!({"output_tokens": out}));
                            }
                            check_stop(&v["delta"]["stop_reason"], &v["delta"]["stop_details"])?
                        }
                        Some("error") => bail!(
                            "Claude API error: {}",
                            v["error"]["message"].as_str().unwrap_or("unknown")
                        ),
                        _ => {}
                    }
                }
            }
        }
        Ok(full)
    }
}

fn check_stop(stop_reason: &Value, details: &Value) -> Result<()> {
    if stop_reason == "refusal" {
        let why = details["explanation"].as_str().unwrap_or("no explanation given");
        return Err(anyhow!("Claude declined this request ({why})"));
    }
    Ok(())
}

fn api_error(status: u16, v: &Value) -> String {
    let msg = v["error"]["message"].as_str().unwrap_or("unknown error");
    match status {
        401 => "Claude API rejected the API key (401). Check it in Settings.".into(),
        429 => format!("Rate limited by the Claude API (429): {msg}"),
        _ => format!("Claude API error {status}: {msg}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_usage() {
        let mut u = Usage::default();
        u.add_from(&json!({"input_tokens": 1_000_000, "output_tokens": 100_000, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}));
        assert_eq!((u.input, u.output), (1_000_000, 100_000));
        assert!((u.cost("claude-haiku-5-5").unwrap() - 0.15).abs() < 1e-9);
        assert!((u.cost("claude-opus-5-5").unwrap() - 6.0).abs() < 1e-9);
        assert_eq!(u.cost("some-other-model"), None);
    }
}
