//! Shared app state, used by both the Tauri commands and the MCP server.

use crate::claude::Claude;
use crate::db;
use anyhow::{Result, anyhow};
use rusqlite::Connection;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

const KEYRING_SERVICE: &str = "pdf-tutor";
const KEYRING_USER: &str = "anthropic-api-key";

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

    pub fn api_key(&self) -> Option<String> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .ok()
            .and_then(|e| e.get_password().ok())
            .filter(|k| !k.is_empty())
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.is_empty()))
    }

    pub fn set_api_key(&self, key: &str) -> Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
        if key.is_empty() {
            let _ = entry.delete_credential();
        } else {
            entry.set_password(key)?;
        }
        Ok(())
    }

    pub fn claude(&self) -> Result<Claude> {
        let key = self
            .api_key()
            .ok_or_else(|| anyhow!("No Anthropic API key set. Add one in Settings."))?;
        let model = self.with_db(|c| db::get_setting(c, "model"))?;
        Ok(Claude::new(key, model))
    }
}
