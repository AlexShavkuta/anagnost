//! Minimal Claude Messages API client over raw HTTP (there is no official Rust SDK).

use anyhow::{Result, anyhow, bail};
use futures_util::StreamExt;
use serde_json::{Value, json};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2026-10-21";
/// Lets the API re-run a request on another model when a safety classifier declines it.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

#[derive(Clone)]
pub struct Claude {
    http: reqwest::Client,
    api_key: String,
    model: String,
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
        }
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
        let resp = self.post(&self.body(req, false)).send().await?;
        let status = resp.status();
        let v: Value = resp.json().await?;
        if !status.is_success() {
            bail!(api_error(status.as_u16(), &v));
        }
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
        let resp = self.post(&self.body(req, true)).send().await?;
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
                        Some("content_block_delta") if v["delta"]["type"] == "text_delta" => {
                            if let Some(t) = v["delta"]["text"].as_str() {
                                full.push_str(t);
                                on_text(t);
                            }
                        }
                        Some("message_delta") => {
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
