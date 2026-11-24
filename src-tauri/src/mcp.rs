//! MCP server (Streamable HTTP transport, JSON responses) on 127.0.0.1.
//! Claude Code connects with:
//!   claude mcp add --transport http anagnost http://127.0.0.1:<port>/mcp \
//!     --header "Authorization: Bearer <token>"

use crate::db::{self, Concept};
use crate::state::AppState;
use anyhow::{Result, anyhow};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::Emitter;

const SERVER_NAME: &str = "anagnost";
const DEFAULT_PROTOCOL: &str = "2025-06-18";
const MAX_PAGES_PER_READ: i64 = 30;

pub async fn serve(state: Arc<AppState>) -> Result<()> {
    let addr = format!("127.0.0.1:{}", state.mcp_port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| anyhow!("could not listen on {addr}: {e}"))?;
    let app = Router::new()
        .route("/mcp", post(handle).get(not_allowed).delete(not_allowed))
        .with_state(state);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn not_allowed() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

async fn handle(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    // Browsers always send Origin; Claude Code doesn't. Rejecting it blocks web pages
    // (including DNS-rebinding attacks) from talking to this server.
    if headers.contains_key("origin") {
        return (StatusCode::FORBIDDEN, "browser requests are not allowed").into_response();
    }
    let expected = format!("Bearer {}", state.mcp_token);
    if headers.get("authorization").and_then(|h| h.to_str().ok()) != Some(expected.as_str()) {
        return (StatusCode::UNAUTHORIZED, "missing or wrong bearer token").into_response();
    }

    match body {
        Value::Array(msgs) => {
            let out: Vec<Value> = msgs.into_iter().filter_map(|m| dispatch(&state, m)).collect();
            if out.is_empty() { StatusCode::ACCEPTED.into_response() } else { Json(out).into_response() }
        }
        msg => match dispatch(&state, msg) {
            Some(resp) => Json(resp).into_response(),
            None => StatusCode::ACCEPTED.into_response(), // notification
        },
    }
}

/// Returns `None` for notifications (messages without an id).
fn dispatch(state: &AppState, msg: Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let method = msg["method"].as_str().unwrap_or("");
    let params = &msg["params"];
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(DEFAULT_PROTOCOL),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Tools for the PDF the user has open in Anagnost. Start with get_status, \
                then get_map to see the section tree. Use read_section to read text, go_to_page to \
                show the user a page, and save_section_analysis / add_note to record what you teach \
                so it appears in the app's map and notes.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(match call_tool(state, name, &args) {
                Ok(text) => json!({"content": [{"type": "text", "text": text}]}),
                Err(e) => json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true}),
            })
        }
        _ => Err(json!({"code": -32601, "message": format!("method not found: {method}")})),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    })
}

fn tools() -> Value {
    let obj = |props: Value, required: &[&str]| {
        json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
    };
    json!([
        {
            "name": "get_status",
            "description": "The open document (title, page count), the page the user is on, their current text selection, and how much of the study map is built.",
            "inputSchema": obj(json!({}), &[]),
        },
        {
            "name": "get_map",
            "description": "The document's section tree with ids, page ranges, studied flags, and summaries/concepts where they exist. Pass include_details=false for just titles and pages.",
            "inputSchema": obj(json!({"include_details": {"type": "boolean"}}), &[]),
        },
        {
            "name": "read_section",
            "description": "Full extracted text of one section (by id from get_map), page-tagged. Long sections are capped at 30 pages; use read_pages for the rest.",
            "inputSchema": obj(json!({"section_id": {"type": "integer"}}), &["section_id"]),
        },
        {
            "name": "read_pages",
            "description": "Extracted text of a page range (1-based, inclusive, at most 30 pages).",
            "inputSchema": obj(json!({"start": {"type": "integer"}, "end": {"type": "integer"}}), &["start", "end"]),
        },
        {
            "name": "search",
            "description": "Case-insensitive text search across the document. Returns pages with snippets.",
            "inputSchema": obj(json!({"query": {"type": "string"}}), &["query"]),
        },
        {
            "name": "go_to_page",
            "description": "Scroll the user's viewer to a page, e.g. to show them what you're explaining.",
            "inputSchema": obj(json!({"page": {"type": "integer"}}), &["page"]),
        },
        {
            "name": "save_section_analysis",
            "description": "Store a summary, key concepts and review questions for a section. They appear in the app's map. Use this to build or improve the map.",
            "inputSchema": obj(json!({
                "section_id": {"type": "integer"},
                "summary": {"type": "string"},
                "concepts": {"type": "array", "items": obj(json!({"term": {"type": "string"}, "explanation": {"type": "string"}}), &["term", "explanation"])},
                "questions": {"type": "array", "items": {"type": "string"}},
            }), &["section_id", "summary", "concepts", "questions"]),
        },
        {
            "name": "set_overview",
            "description": "Store the whole-document study overview (Markdown) shown at the top of the map.",
            "inputSchema": obj(json!({"overview": {"type": "string"}}), &["overview"]),
        },
        {
            "name": "mark_studied",
            "description": "Mark a section as studied (or not) to track the user's progress.",
            "inputSchema": obj(json!({"section_id": {"type": "integer"}, "studied": {"type": "boolean"}}), &["section_id", "studied"]),
        },
        {
            "name": "add_note",
            "description": "Add a note to the document's notes panel, optionally tied to a page.",
            "inputSchema": obj(json!({"text": {"type": "string"}, "page": {"type": "integer"}}), &["text"]),
        },
        {
            "name": "list_notes",
            "description": "All notes on the open document, newest first.",
            "inputSchema": obj(json!({}), &[]),
        },
    ])
}

fn int(args: &Value, key: &str) -> Result<i64> {
    args[key].as_i64().ok_or_else(|| anyhow!("missing integer argument `{key}`"))
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args[key].as_str().ok_or_else(|| anyhow!("missing string argument `{key}`"))
}

fn call_tool(state: &AppState, name: &str, args: &Value) -> Result<String> {
    let doc_id = state.current_doc()?;
    let pretty = |v: Value| serde_json::to_string_pretty(&v).unwrap_or_default();
    match name {
        "get_status" => {
            let view = state.view.lock().unwrap().clone();
            let (doc, secs, text) = state.with_db(|c| {
                Ok((db::get_document(c, &doc_id)?, db::sections(c, &doc_id)?, db::text_status(c, &doc_id)?))
            })?;
            let doc = doc.ok_or_else(|| anyhow!("document not found"))?;
            let mapped = secs.iter().filter(|s| s.summary.is_some()).count();
            Ok(pretty(json!({
                "pages_without_text": text.missing.len(),
                "pages_recognized_with_ocr": text.recognized.len(),
                "title": doc.title,
                "path": doc.path,
                "page_count": doc.page_count,
                "current_page": view.page,
                "selection": view.selection,
                "sections": secs.len(),
                "sections_with_summary": mapped,
                "has_overview": doc.overview.is_some(),
            })))
        }
        "get_map" => {
            let details = args["include_details"].as_bool().unwrap_or(true);
            let (doc, secs) = state.with_db(|c| Ok((db::get_document(c, &doc_id)?, db::sections(c, &doc_id)?)))?;
            let secs: Vec<Value> = secs
                .iter()
                .map(|s| {
                    let mut v = json!({
                        "id": s.id, "parent_id": s.parent_id, "depth": s.depth, "title": s.title,
                        "pages": format!("{}-{}", s.start_page, s.end_page), "studied": s.studied,
                    });
                    if details {
                        v["summary"] = json!(s.summary);
                        v["concepts"] = json!(s.concepts);
                        v["questions"] = json!(s.questions);
                    }
                    v
                })
                .collect();
            Ok(pretty(json!({"overview": doc.and_then(|d| d.overview), "sections": secs})))
        }
        "read_section" => {
            let id = int(args, "section_id")?;
            state.with_db(|c| {
                let s = db::section(c, &doc_id, id)?.ok_or_else(|| anyhow!("no section with id {id}"))?;
                let end = s.end_page.min(s.start_page + MAX_PAGES_PER_READ - 1);
                let mut out = format!("# {} (pages {}–{})\n\n", s.title, s.start_page, s.end_page);
                out.push_str(&db::page_text(c, &doc_id, s.start_page, end)?);
                if end < s.end_page {
                    out.push_str(&format!("\n[truncated at page {end}; call read_pages for {}–{}]", end + 1, s.end_page));
                }
                Ok(out)
            })
        }
        "read_pages" => {
            let start = int(args, "start")?.max(1);
            let end = int(args, "end")?.min(start + MAX_PAGES_PER_READ - 1);
            state.with_db(|c| db::page_text(c, &doc_id, start, end))
        }
        "search" => {
            let q = str_arg(args, "query")?;
            let hits = state.with_db(|c| db::search(c, &doc_id, q))?;
            Ok(if hits.is_empty() { format!("No matches for \"{q}\".") } else { pretty(json!(hits)) })
        }
        "go_to_page" => {
            let page = int(args, "page")?;
            state.app.emit("goto-page", page)?;
            Ok(format!("Showing page {page}."))
        }
        "save_section_analysis" => {
            let id = int(args, "section_id")?;
            let summary = str_arg(args, "summary")?;
            let concepts: Vec<Concept> = serde_json::from_value(args["concepts"].clone())?;
            let questions: Vec<String> = serde_json::from_value(args["questions"].clone())?;
            let ok = state.with_db(|c| db::save_section_analysis(c, &doc_id, id, summary, &concepts, &questions))?;
            if !ok {
                return Err(anyhow!("no section with id {id}"));
            }
            state.notify_changed(&doc_id);
            Ok(format!("Saved analysis for section {id}."))
        }
        "set_overview" => {
            let text = str_arg(args, "overview")?;
            state.with_db(|c| db::set_overview(c, &doc_id, text))?;
            state.notify_changed(&doc_id);
            Ok("Overview saved.".into())
        }
        "mark_studied" => {
            let id = int(args, "section_id")?;
            let studied = args["studied"].as_bool().unwrap_or(true);
            state.with_db(|c| db::set_studied(c, &doc_id, id, studied))?;
            state.notify_changed(&doc_id);
            Ok(format!("Section {id} marked {}.", if studied { "studied" } else { "not studied" }))
        }
        "add_note" => {
            let text = str_arg(args, "text")?;
            let page = args["page"].as_i64();
            let id = state.with_db(|c| db::add_note(c, &doc_id, page, text, "claude-code"))?;
            state.notify_changed(&doc_id);
            Ok(format!("Note {id} added."))
        }
        "list_notes" => Ok(pretty(json!(state.with_db(|c| db::notes(c, &doc_id))?))),
        _ => Err(anyhow!("unknown tool: {name}")),
    }
}
