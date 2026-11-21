mod claude;
mod db;
mod mapping;
mod mcp;
mod state;

use claude::Request;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use state::{AppState, ViewState};
use std::sync::{Arc, Mutex};
use tauri::{Manager, State, ipc::Channel};

const DEFAULT_MCP_PORT: u16 = 7337;

type S<'a> = State<'a, Arc<AppState>>;
type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---------- documents ----------

#[tauri::command]
fn read_file(path: String) -> CmdResult<tauri::ipc::Response> {
    std::fs::read(&path).map(tauri::ipc::Response::new).map_err(err)
}

#[derive(Serialize)]
struct Opened {
    id: String,
    /// Already ingested; the frontend can skip text extraction.
    document: Option<db::Document>,
}

#[tauri::command]
fn open_document(state: S, path: String) -> CmdResult<Opened> {
    let bytes = std::fs::read(&path).map_err(err)?;
    let id = hex::encode(Sha256::digest(&bytes));
    let document = state
        .with_db(|c| {
            let d = db::get_document(c, &id)?;
            if d.is_some() {
                db::touch_document(c, &id, &path)?;
            }
            Ok(d)
        })
        .map_err(err)?;
    Ok(Opened { id, document })
}

#[tauri::command]
fn ingest_document(
    state: S,
    id: String,
    path: String,
    title: String,
    pages: Vec<String>,
    outline: Vec<db::OutlineItem>,
) -> CmdResult<db::Document> {
    state
        .with_db(|c| {
            db::ingest(c, &id, &path, &title, &pages, &outline)?;
            db::get_document(c, &id)?.ok_or_else(|| anyhow::anyhow!("ingest failed"))
        })
        .map_err(err)
}

#[tauri::command]
fn recent_documents(state: S) -> CmdResult<Vec<db::Document>> {
    state.with_db(|c| db::recent_documents(c)).map_err(err)
}

#[tauri::command]
fn get_document(state: S, id: String) -> CmdResult<Option<db::Document>> {
    state.with_db(|c| db::get_document(c, &id)).map_err(err)
}

#[tauri::command]
fn get_sections(state: S, doc_id: String) -> CmdResult<Vec<db::Section>> {
    state.with_db(|c| db::sections(c, &doc_id)).map_err(err)
}

#[tauri::command]
fn set_studied(state: S, doc_id: String, section_id: i64, studied: bool) -> CmdResult<()> {
    state.with_db(|c| db::set_studied(c, &doc_id, section_id, studied)).map_err(err)
}

/// The frontend reports what the user is looking at, so the MCP tools can see it.
#[tauri::command]
fn set_view(state: S, doc_id: Option<String>, page: i64, selection: Option<String>) -> CmdResult<()> {
    if let Some(id) = &doc_id {
        state.with_db(|c| db::set_last_page(c, id, page)).map_err(err)?;
    }
    *state.view.lock().unwrap() = ViewState { doc_id, page, selection };
    Ok(())
}

// ---------- Claude ----------

#[tauri::command]
async fn build_map(state: S<'_>, doc_id: String, redo: bool) -> CmdResult<()> {
    mapping::build_map(state.inner().clone(), doc_id, redo).await.map_err(err)
}

#[derive(Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

/// Chat about the document, grounded in one section (or the pages around `page`).
/// Streams text deltas over `on_delta`; returns the full reply.
#[tauri::command]
async fn ask(
    state: S<'_>,
    doc_id: String,
    section_id: Option<i64>,
    page: i64,
    messages: Vec<ChatMessage>,
    on_delta: Channel<String>,
) -> CmdResult<String> {
    let claude = state.claude().map_err(err)?;
    let context = state
        .with_db(|c| {
            let doc = db::get_document(c, &doc_id)?.ok_or_else(|| anyhow::anyhow!("unknown document"))?;
            let sec = match section_id {
                Some(id) => db::section(c, &doc_id, id)?,
                None => None,
            };
            let (label, start, end) = match &sec {
                // Cap very long sections at 40 pages around the reader's position.
                Some(s) if s.end_page - s.start_page > 40 => {
                    let st = (page - 20).max(s.start_page);
                    (format!("section \"{}\" (excerpt)", s.title), st, (st + 40).min(s.end_page))
                }
                Some(s) => (format!("section \"{}\"", s.title), s.start_page, s.end_page),
                None => ("the pages around the reader".into(), (page - 2).max(1), page + 2),
            };
            let text = db::page_text(c, &doc_id, start, end)?;
            let overview = doc.overview.map(|o| format!("\n\nDocument overview:\n{o}")).unwrap_or_default();
            Ok(format!(
                "The reader has \"{}\" open. Below is {label}, pages {start}–{end}.{overview}\n\n<document_text>\n{text}\n</document_text>",
                doc.title
            ))
        })
        .map_err(err)?;

    let system = format!(
        "{}\n\nThe reader is currently on page {page}. Answer from the provided text and cite pages like (p. 12). \
         Keep answers focused; use short paragraphs and Markdown where it helps.",
        mapping::SYSTEM
    );
    let req = Request {
        system: vec![system, context],
        messages: messages.iter().map(|m| json!({"role": m.role, "content": m.content})).collect(),
        max_tokens: 64000,
        effort: "medium",
        schema: None,
    };
    claude
        .stream(&req, |t| {
            let _ = on_delta.send(t.to_string());
        })
        .await
        .map_err(err)
}

// ---------- notes ----------

#[tauri::command]
fn add_note(state: S, doc_id: String, page: Option<i64>, text: String, source: String) -> CmdResult<i64> {
    state.with_db(|c| db::add_note(c, &doc_id, page, &text, &source)).map_err(err)
}

#[tauri::command]
fn list_notes(state: S, doc_id: String) -> CmdResult<Vec<db::Note>> {
    state.with_db(|c| db::notes(c, &doc_id)).map_err(err)
}

#[tauri::command]
fn delete_note(state: S, doc_id: String, id: i64) -> CmdResult<()> {
    state.with_db(|c| db::delete_note(c, &doc_id, id)).map_err(err)
}

// ---------- settings ----------

#[derive(Serialize)]
struct Settings {
    has_api_key: bool,
    model: String,
    mcp_url: String,
    mcp_token: String,
    mcp_error: Option<String>,
}

#[tauri::command]
fn get_settings(state: S) -> CmdResult<Settings> {
    let model = state
        .with_db(|c| db::get_setting(c, "model"))
        .map_err(err)?
        .unwrap_or_else(|| claude::DEFAULT_MODEL.into());
    Ok(Settings {
        has_api_key: state.api_key().is_some(),
        model,
        mcp_url: format!("http://127.0.0.1:{}/mcp", state.mcp_port),
        mcp_token: state.mcp_token.clone(),
        mcp_error: state.mcp_error.lock().unwrap().clone(),
    })
}

#[tauri::command]
fn save_settings(state: S, api_key: Option<String>, model: Option<String>) -> CmdResult<()> {
    if let Some(k) = api_key {
        state.set_api_key(k.trim()).map_err(err)?;
    }
    if let Some(m) = model {
        state.with_db(|c| db::set_setting(c, "model", m.trim())).map_err(err)?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let conn = db::open(&dir.join("library.db"))?;

            let token = match db::get_setting(&conn, "mcp_token")? {
                Some(t) => t,
                None => {
                    let t = uuid::Uuid::new_v4().simple().to_string();
                    db::set_setting(&conn, "mcp_token", &t)?;
                    t
                }
            };
            let port = db::get_setting(&conn, "mcp_port")?
                .and_then(|p| p.parse().ok())
                .unwrap_or(DEFAULT_MCP_PORT);

            let state = Arc::new(AppState {
                db: Mutex::new(conn),
                view: Mutex::new(ViewState::default()),
                app: app.handle().clone(),
                mcp_port: port,
                mcp_token: token,
                mcp_error: Mutex::new(None),
            });
            app.manage(state.clone());

            tauri::async_runtime::spawn(async move {
                if let Err(e) = mcp::serve(state.clone()).await {
                    eprintln!("MCP server stopped: {e}");
                    *state.mcp_error.lock().unwrap() = Some(e.to_string());
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            read_file,
            open_document,
            ingest_document,
            recent_documents,
            get_document,
            get_sections,
            set_studied,
            set_view,
            build_map,
            ask,
            add_note,
            list_notes,
            delete_note,
            get_settings,
            save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
