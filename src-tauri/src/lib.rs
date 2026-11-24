mod claude;
mod cursor;
mod db;
mod llm;
mod mapping;
mod mcp;
mod secrets;
mod ocr;
mod state;
mod update;

use claude::Request;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use state::{AppState, ViewState};
use std::sync::{Arc, Mutex};
use tauri::{Manager, RunEvent, State, ipc::Channel};

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

#[tauri::command]
fn search(state: S, doc_id: String, query: String) -> CmdResult<Vec<db::SearchHit>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(vec![]);
    }
    state.with_db(|c| db::search(c, &doc_id, &query.to_lowercase())).map_err(err)
}

// ---------- OCR ----------

/// On unless turned off in Settings.
fn ocr_autoload(state: &AppState) -> bool {
    state.setting("ocr_autoload").as_deref() != Some("0")
}

fn llama_server_path(state: &AppState) -> Option<String> {
    state.with_db(|c| db::get_setting(c, "llama_server_path")).ok().flatten()
}

#[tauri::command]
fn text_status(state: S, doc_id: String) -> CmdResult<db::TextStatus> {
    state.with_db(|c| db::text_status(c, &doc_id)).map_err(err)
}

#[tauri::command]
fn ocr_info(state: S) -> ocr::Info {
    state.ocr.info(llama_server_path(&state).as_deref())
}

/// Recognize one rendered page (base64 PNG) and store its text.
#[tauri::command]
async fn ocr_page(state: S<'_>, doc_id: String, page: i64, image: String) -> CmdResult<String> {
    let path = llama_server_path(&state);
    let text = state.ocr.recognize(path.as_deref(), &image).await.map_err(err)?;
    state.with_db(|c| db::set_ocr_text(c, &doc_id, page, &text)).map_err(err)?;
    Ok(text)
}

/// Called after a run of OCR: re-split generated sections and tell views to refresh.
#[tauri::command]
fn ocr_finish(state: S, doc_id: String) -> CmdResult<bool> {
    let rechunked = state.with_db(|c| db::rechunk_if_generated(c, &doc_id)).map_err(err)?;
    state.notify_changed(&doc_id);
    Ok(rechunked)
}

/// Models the saved Cursor key can use, for the Model suggestions in Settings.
#[tauri::command]
async fn cursor_models(state: S<'_>) -> CmdResult<Vec<String>> {
    let cursor = state.cursor().map_err(err)?;
    cursor.models().await.map_err(err)
}

/// Appends the frontend's performance events to perf.log in the data folder, kept under 512 KB.
#[tauri::command]
fn perf_log(app: tauri::AppHandle, lines: Vec<String>) -> CmdResult<()> {
    use std::io::Write;
    let path = app.path().app_data_dir().map_err(err)?.join("perf.log");
    if std::fs::metadata(&path).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut half = text.len() / 2;
        while !text.is_char_boundary(half) {
            half += 1;
        }
        let keep = &text[half..];
        let keep = keep.find('\n').map_or(keep, |i| &keep[i + 1..]);
        std::fs::write(&path, keep).map_err(err)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).map_err(err)?;
    for l in lines.iter().take(1000) {
        writeln!(f, "{}", l.replace('\n', " ")).map_err(err)?;
    }
    Ok(())
}

// ---------- updates ----------

#[tauri::command]
fn update_status(state: S) -> update::Status {
    state.updater.status()
}

#[tauri::command]
async fn check_update(state: S<'_>) -> CmdResult<update::Check> {
    state.updater.check().await.map_err(err)
}

#[tauri::command]
fn start_update(state: S) -> CmdResult<()> {
    state.updater.start().map_err(err)
}

/// Install llama.cpp if needed, then download and load the model.
#[tauri::command]
async fn ocr_setup(state: S<'_>) -> CmdResult<()> {
    let path = llama_server_path(&state);
    state.ocr.setup(path.as_deref()).await.map_err(err)
}

#[tauri::command]
async fn ocr_stop(state: S<'_>) -> CmdResult<()> {
    state.ocr.stop().await;
    Ok(())
}

// ---------- Claude ----------

#[tauri::command]
async fn build_map(state: S<'_>, doc_id: String, redo: bool) -> CmdResult<mapping::Outcome> {
    mapping::build_map(state.inner().clone(), doc_id, redo).await.map_err(|e| format!("{e:#}"))
}

/// Sections, tokens and estimated cost of building the map, shown before starting.
#[tauri::command]
fn map_estimate(state: S, doc_id: String, redo: bool) -> CmdResult<mapping::Estimate> {
    mapping::estimate(&state, &doc_id, redo).map_err(err)
}

/// Stop building the map; sections finished so far are kept.
#[tauri::command]
fn stop_map(state: S) {
    state.map_stop.store(true, std::sync::atomic::Ordering::SeqCst);
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
    let claude = state.llm().map_err(err)?;
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
    provider: state::Provider,
    has_api_key: bool,
    model: String,
    has_cursor_key: bool,
    cursor_model: String,
    cursor_agent_path: String,
    /// Where cursor-agent was found, if it was.
    cursor_agent: Option<String>,
    mcp_url: String,
    mcp_token: String,
    mcp_error: Option<String>,
    llama_server_path: String,
    /// Load the OCR model when the app opens and keep it loaded.
    ocr_autoload: bool,
    /// Claude model for building the map.
    map_model: String,
}

#[tauri::command]
fn get_settings(state: S) -> CmdResult<Settings> {
    let cursor_agent_path = state.setting("cursor_agent_path").unwrap_or_default();
    Ok(Settings {
        provider: state.provider(),
        has_api_key: state.api_key().is_some(),
        model: state.setting("model").unwrap_or_else(|| claude::DEFAULT_MODEL.into()),
        has_cursor_key: state.cursor_key().is_some(),
        cursor_model: state.setting("cursor_model").unwrap_or_default(),
        cursor_agent: cursor::find_binary(Some(&cursor_agent_path)).map(|p| p.display().to_string()),
        cursor_agent_path,
        mcp_url: format!("http://127.0.0.1:{}/mcp", state.mcp_port),
        mcp_token: state.mcp_token.clone(),
        mcp_error: state.mcp_error.lock().unwrap().clone(),
        llama_server_path: llama_server_path(&state).unwrap_or_default(),
        ocr_autoload: ocr_autoload(&state),
        map_model: state.setting("map_model").unwrap_or_else(|| claude::DEFAULT_MAP_MODEL.into()),
    })
}

/// Fields left as None are unchanged. An empty key string removes that key.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsUpdate {
    provider: Option<state::Provider>,
    api_key: Option<String>,
    model: Option<String>,
    cursor_api_key: Option<String>,
    cursor_model: Option<String>,
    cursor_agent_path: Option<String>,
    llama_server_path: Option<String>,
    ocr_autoload: Option<bool>,
    map_model: Option<String>,
}

#[tauri::command]
fn save_settings(state: S, update: SettingsUpdate) -> CmdResult<()> {
    if let Some(k) = update.api_key {
        state.set_api_key(k.trim()).map_err(err)?;
    }
    if let Some(k) = update.cursor_api_key {
        state.set_cursor_key(k.trim()).map_err(err)?;
    }
    if let Some(on) = update.ocr_autoload {
        state.ocr.set_keep_loaded(on);
        if on {
            let st = state.inner().clone();
            tauri::async_runtime::spawn(async move {
                let path = llama_server_path(&st);
                st.ocr.autoload(path.as_deref()).await;
            });
        }
    }
    let ocr_autoload = update.ocr_autoload.map(|on| if on { "1" } else { "0" }.to_string());
    let provider = update.provider.map(|p| if p == state::Provider::Cursor { "cursor" } else { "anthropic" }.to_string());
    let fields = [
        ("provider", provider),
        ("model", update.model),
        ("cursor_model", update.cursor_model),
        ("cursor_agent_path", update.cursor_agent_path),
        ("llama_server_path", update.llama_server_path),
        ("ocr_autoload", ocr_autoload),
        ("map_model", update.map_model),
    ];
    state
        .with_db(|c| {
            for (key, value) in fields {
                if let Some(v) = value {
                    db::set_setting(c, key, v.trim())?;
                }
            }
            Ok(())
        })
        .map_err(err)
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
                ocr: ocr::Ocr::new(app.handle().clone(), dir.clone()),
                updater: update::Updater::new(app.handle().clone(), &dir),
                secrets: secrets::Secrets::open(&dir)?,
                map_stop: std::sync::atomic::AtomicBool::new(false),
            });
            app.manage(state.clone());
            update::adopt_new_bundle_name();

            if ocr_autoload(&state) {
                state.ocr.set_keep_loaded(true);
                let st = state.clone();
                tauri::async_runtime::spawn(async move {
                    // Let the window come up first; loading reads ~5 GB from disk.
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let path = llama_server_path(&st);
                    st.ocr.autoload(path.as_deref()).await;
                });
            }

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
            map_estimate,
            stop_map,
            ask,
            add_note,
            list_notes,
            delete_note,
            search,
            text_status,
            ocr_info,
            ocr_page,
            ocr_finish,
            ocr_setup,
            cursor_models,
            perf_log,
            update_status,
            check_update,
            start_update,
            ocr_stop,
            get_settings,
            save_settings,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Don't leave the OCR model server running after the app quits.
            if let RunEvent::Exit = event
                && let Some(state) = app.try_state::<Arc<AppState>>()
            {
                state.ocr.kill_now();
            }
        });
}
