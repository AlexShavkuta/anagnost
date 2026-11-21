//! Builds the study map: summarize leaf sections from their text, roll summaries up
//! to parent sections, then write a whole-document overview.

use crate::claude::{Claude, Request};
use crate::db::{self, Concept, Section};
use crate::state::AppState;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::Semaphore;

/// About 100k tokens; sections longer than this are sent in part (and the prompt says so).
const MAX_SECTION_CHARS: usize = 400_000;
const PARALLEL_REQUESTS: usize = 4;

pub const SYSTEM: &str = "You are a patient tutor inside a PDF reader, helping one reader learn \
the document they have open. Be accurate to the text: when something isn't in the provided text, \
say so instead of guessing. Write plainly, define jargon the first time it appears, and prefer \
concrete examples from the document itself.";

#[derive(Deserialize)]
struct Analysis {
    summary: String,
    concepts: Vec<Concept>,
    questions: Vec<String>,
}

fn analysis_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "summary": {"type": "string"},
            "concepts": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "term": {"type": "string"},
                        "explanation": {"type": "string"}
                    },
                    "required": ["term", "explanation"],
                    "additionalProperties": false
                }
            },
            "questions": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["summary", "concepts", "questions"],
        "additionalProperties": false
    })
}

#[derive(Serialize, Clone)]
struct Progress {
    doc_id: String,
    done: usize,
    total: usize,
    current: String,
}

pub async fn build_map(state: Arc<AppState>, doc_id: String, redo: bool) -> Result<()> {
    let claude = state.claude()?;
    let (doc, sections) = state.with_db(|c| {
        let doc = db::get_document(c, &doc_id)?.ok_or_else(|| anyhow!("unknown document"))?;
        Ok((doc, db::sections(c, &doc_id)?))
    })?;

    let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
    for s in &sections {
        if let Some(p) = s.parent_id {
            children.entry(p).or_default().push(s.id);
        }
    }
    let todo = |s: &Section| redo || s.summary.is_none();
    let leaves: Vec<Section> =
        sections.iter().filter(|s| !children.contains_key(&s.id) && todo(s)).cloned().collect();
    let mut parents: Vec<Section> =
        sections.iter().filter(|s| children.contains_key(&s.id) && todo(s)).cloned().collect();
    parents.sort_by_key(|s| -s.depth); // deepest first, so children are done before parents

    let total = leaves.len() + parents.len() + 1;
    let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let progress = |current: &str| {
        let _ = state.app.emit(
            "map-progress",
            Progress {
                doc_id: doc_id.clone(),
                done: done.load(std::sync::atomic::Ordering::SeqCst),
                total,
                current: current.to_string(),
            },
        );
    };
    progress("Starting");

    // 1. Leaves, in parallel.
    let sem = Arc::new(Semaphore::new(PARALLEL_REQUESTS));
    let mut tasks = Vec::new();
    for s in leaves {
        let (state, claude, sem, done) = (state.clone(), claude.clone(), sem.clone(), done.clone());
        let (doc_id, title) = (doc_id.clone(), doc.title.clone());
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await?;
            let text = state.with_db(|c| db::page_text(c, &doc_id, s.start_page, s.end_page))?;
            let a = analyze_leaf(&claude, &title, &s, &text).await?;
            state.with_db(|c| {
                db::save_section_analysis(c, &doc_id, s.id, &a.summary, &a.concepts, &a.questions)
            })?;
            done.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            state.notify_changed(&doc_id);
            Ok::<String, anyhow::Error>(s.title)
        }));
    }
    let mut first_err = None;
    for t in tasks {
        match t.await? {
            Ok(title) => progress(&title),
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    if let Some(e) = first_err {
        return Err(e);
    }

    // 2. Parents, from their children's summaries.
    for p in parents {
        progress(&p.title);
        let kids: Vec<Section> = state.with_db(|c| {
            Ok(db::sections(c, &doc_id)?.into_iter().filter(|s| s.parent_id == Some(p.id)).collect())
        })?;
        let a = analyze_parent(&claude, &doc.title, &p, &kids).await?;
        state.with_db(|c| db::save_section_analysis(c, &doc_id, p.id, &a.summary, &a.concepts, &a.questions))?;
        done.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        state.notify_changed(&doc_id);
    }

    // 3. Whole-document overview.
    progress("Document overview");
    let top: Vec<Section> = state.with_db(|c| {
        Ok(db::sections(c, &doc_id)?.into_iter().filter(|s| s.parent_id.is_none()).collect())
    })?;
    let overview = overview(&claude, &doc.title, doc.page_count, &top).await?;
    state.with_db(|c| db::set_overview(c, &doc_id, &overview))?;
    done.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    state.notify_changed(&doc_id);
    progress("Done");
    Ok(())
}

async fn analyze_leaf(claude: &Claude, doc_title: &str, s: &Section, text: &str) -> Result<Analysis> {
    let (text, note) = if text.len() > MAX_SECTION_CHARS {
        let mut cut = MAX_SECTION_CHARS;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        (&text[..cut], "\nNote: this section is very long; only its first part is included above.")
    } else {
        (text, "")
    };
    let prompt = format!(
        "Document: {doc_title}\nSection: {} (pages {}–{})\n\n<section_text>\n{text}\n</section_text>{note}\n\n\
         Analyze this section for someone studying it:\n\
         - summary: 3–6 sentences covering what the section says and why it matters in the document.\n\
         - concepts: the 3–8 most important terms or ideas, each with a 1–2 sentence explanation grounded in the text.\n\
         - questions: 3–5 review questions that test understanding rather than recall of wording.\n\
         If the text is front matter, an index, or references, say so briefly in the summary and keep the lists short.",
        s.title, s.start_page, s.end_page
    );
    run(claude, prompt).await
}

async fn analyze_parent(claude: &Claude, doc_title: &str, p: &Section, kids: &[Section]) -> Result<Analysis> {
    let mut parts = String::new();
    for k in kids {
        parts.push_str(&format!(
            "## {} (pages {}–{})\n{}\nKey concepts: {}\n\n",
            k.title,
            k.start_page,
            k.end_page,
            k.summary.as_deref().unwrap_or("(no summary)"),
            k.concepts.iter().map(|c| c.term.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    let prompt = format!(
        "Document: {doc_title}\nPart: {} (pages {}–{})\n\nIts subsections, already summarized:\n\n{parts}\
         Synthesize this part as a whole:\n\
         - summary: 4–7 sentences on the part's main argument and how its subsections build on each other.\n\
         - concepts: the 4–8 ideas that matter most across the whole part.\n\
         - questions: 3–5 questions that connect ideas across subsections.",
        p.title, p.start_page, p.end_page
    );
    run(claude, prompt).await
}

async fn overview(claude: &Claude, title: &str, pages: i64, top: &[Section]) -> Result<String> {
    let mut parts = String::new();
    for s in top {
        parts.push_str(&format!(
            "## {} (pages {}–{})\n{}\n\n",
            s.title,
            s.start_page,
            s.end_page,
            s.summary.as_deref().unwrap_or("(no summary)")
        ));
    }
    let prompt = format!(
        "Document: {title} ({pages} pages)\n\nTop-level parts, summarized:\n\n{parts}\
         Write a study overview in Markdown for someone about to learn this document:\n\
         1. What the document is about and who it is for (2–3 sentences).\n\
         2. The core ideas, as a short bullet list.\n\
         3. A suggested reading path: which parts to read first, which depend on which, and what can be skimmed.\n\
         Keep it under 350 words."
    );
    claude
        .complete(&Request {
            system: vec![SYSTEM.into()],
            messages: vec![json!({"role": "user", "content": prompt})],
            max_tokens: 16000,
            effort: "medium",
            schema: None,
        })
        .await
}

async fn run(claude: &Claude, prompt: String) -> Result<Analysis> {
    let text = claude
        .complete(&Request {
            system: vec![SYSTEM.into()],
            messages: vec![json!({"role": "user", "content": prompt})],
            max_tokens: 16000,
            effort: "medium",
            schema: Some(analysis_schema()),
        })
        .await?;
    serde_json::from_str(&text).map_err(|e| anyhow!("Claude returned malformed JSON: {e}"))
}
