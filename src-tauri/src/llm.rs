//! The model behind the tutor and the map: the Claude API with the user's Anthropic key, or
//! Cursor (cursor-agent) with the user's Cursor key. Chosen in Settings.

use crate::claude::{Claude, Request};
use crate::cursor::Cursor;
use anyhow::Result;

#[derive(Clone)]
pub enum Llm {
    Claude(Claude),
    Cursor(Cursor),
}

impl Llm {
    pub async fn complete(&self, req: &Request) -> Result<String> {
        match self {
            Llm::Claude(c) => c.complete(req).await,
            Llm::Cursor(c) => c.complete(req).await,
        }
    }

    pub async fn stream(&self, req: &Request, on_text: impl FnMut(&str)) -> Result<String> {
        match self {
            Llm::Claude(c) => c.stream(req, on_text).await,
            Llm::Cursor(c) => c.stream(req, on_text).await,
        }
    }

    /// Parse a structured reply. The Claude API enforces the schema; Cursor is only asked to,
    /// so tolerate a code fence or a sentence around the JSON.
    pub fn parse_json<T: serde::de::DeserializeOwned>(&self, text: &str) -> Result<T> {
        let json = match self {
            Llm::Claude(_) => text,
            Llm::Cursor(_) => crate::cursor::extract_json(text)?,
        };
        serde_json::from_str(json).map_err(|e| anyhow::anyhow!("The model returned malformed JSON: {e}"))
    }
}
