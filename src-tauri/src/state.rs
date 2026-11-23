//! Shared app state, used by both the Tauri commands and the MCP server.

use crate::claude::Claude;
use crate::cursor::Cursor;
use crate::llm::Llm;
use crate::db;
use anyhow::{Result, anyhow};
use rusqlite::Connection;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

const KEYRING_SERVICE: &str = "pdf-tutor";
const KEYRING_USER: &str = "anthropic-api-key";
const KEYRING_CURSOR: &str = "cursor-api-key";

/// Which model provider runs the tutor and the map.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Anthropic,
    Cursor,
}

#[derive(Default, Clone, Serialize)]
pub struct ViewState {
    pub doc_id: Option<String>,
    pub page: i64,
    pub selection: Option<String>,
}

pub struct AppState {
    /// Never hold this lock across an `.await`.
    pub db: Mutex<Connection>,
    pub view: Mutex<ViewState>,
    pub app: AppHandle,
    pub mcp_port: u16,
    pub mcp_token: String,
    pub mcp_error: Mutex<Option<String>>,
    pub ocr: std::sync::Arc<crate::ocr::Ocr>,
}

impl AppState {
    pub fn with_db<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut conn = self.db.lock().map_err(|_| anyhow!("database lock poisoned"))?;
        f(&mut conn)
    }

    pub fn current_doc(&self) -> Result<String> {
        self.view
            .lock()
            .unwrap()
            .doc_id
            .clone()
            .ok_or_else(|| anyhow!("No document is open in PDF Tutor. Ask the user to open one."))
    }

    /// Tell the frontend that stored data for a document changed (map, notes, overview).
    pub fn notify_changed(&self, doc_id: &str) {
        let _ = self.app.emit("doc-changed", doc_id.to_string());
    }

    fn secret(user: &str) -> Option<String> {
        keyring::Entry::new(KEYRING_SERVICE, user)
            .ok()
            .and_then(|e| e.get_password().ok())
            .filter(|k| !k.is_empty())
    }

    fn set_secret(user: &str, key: &str) -> Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, user)?;
        if key.is_empty() {
            let _ = entry.delete_credential();
        } else {
            entry.set_password(key)?;
        }
        Ok(())
    }

    pub fn api_key(&self) -> Option<String> {
        Self::secret(KEYRING_USER).or_else(|| std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.is_empty()))
    }

    pub fn set_api_key(&self, key: &str) -> Result<()> {
        Self::set_secret(KEYRING_USER, key)
    }

    pub fn cursor_key(&self) -> Option<String> {
        Self::secret(KEYRING_CURSOR).or_else(|| std::env::var("CURSOR_API_KEY").ok().filter(|k| !k.is_empty()))
    }

    pub fn set_cursor_key(&self, key: &str) -> Result<()> {
        Self::set_secret(KEYRING_CURSOR, key)
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        self.with_db(|c| db::get_setting(c, key)).ok().flatten().filter(|v| !v.is_empty())
    }

    pub fn provider(&self) -> Provider {
        match self.setting("provider").as_deref() {
            Some("cursor") => Provider::Cursor,
            _ => Provider::Anthropic,
        }
    }

    /// The model client for the tutor and the map, per the provider chosen in Settings.
    pub fn llm(&self) -> Result<Llm> {
        match self.provider() {
            Provider::Anthropic => {
                let key = self
                    .api_key()
                    .ok_or_else(|| anyhow!("No Anthropic API key set. Add one in Settings, or switch the provider to Cursor."))?;
                Ok(Llm::Claude(Claude::new(key, self.setting("model"))))
            }
            Provider::Cursor => {
                let path = self.setting("cursor_agent_path");
                let binary = crate::cursor::find_binary(path.as_deref()).ok_or_else(|| {
                    anyhow!("cursor-agent wasn't found. Install Cursor's CLI (curl https://cursor.com/install -fsS | bash), or set its path in Settings.")
                })?;
                Ok(Llm::Cursor(Cursor::new(binary, self.cursor_key(), self.setting("cursor_model"))))
            }
        }
    }
}
