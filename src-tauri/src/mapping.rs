//! Builds the study map: summarize leaf sections from their text, roll summaries up
//! to parent sections, then write a whole-document overview.

use crate::claude::Request;
use crate::llm::Llm;
use crate::db::{self, Concept, Section};
use crate::state::AppState;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use crate::claude::{Meter, Usage};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use tokio::sync::Semaphore;

/// About 100k tokens; sections longer than this are sent in part (and the prompt says so).
const MAX_SECTION_CHARS: usize = 400_000;
const PARALLEL_REQUESTS: usize = 8;

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
    /// Tokens used so far in this run (Claude only; Cursor doesn't report them).
    usage: Option<Usage>,
    /// Estimated cost so far in US dollars, at list prices.
    cost: Option<f64>,
}

#[derive(Serialize)]
pub struct Outcome {
    pub stopped: bool,
    pub sections: usize,
    pub model: Option<String>,
    pub usage: Option<Usage>,
    pub cost: Option<f64>,
}

#[derive(Serialize)]
pub struct Estimate {
    pub sections: usize,
    /// None when Cursor builds the map (it picks the model and bills your plan).
    pub model: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: Option<f64>,
}

/// What a build would cost, from the text it would send. Rough by nature: thinking length varies.
pub fn estimate(state: &AppState, doc_id: &str, redo: bool) -> Result<Estimate> {
    let llm = state.map_llm()?;
    let sections = state.with_db(|c| db::sections(c, doc_id))?;
    let (leaves, parents) = plan(&sections, redo);
    let mut input = 0u64;
    for s in &leaves {
        let chars = state.with_db(|c| db::page_text(c, doc_id, s.start_page, s.end_page))?.len().min(MAX_SECTION_CHARS);
        input += chars as u64 * 10 / 33 + PROMPT_TOKENS; // ~3.3 characters per token
    }
    let children = |p: &Section| sections.iter().filter(|s| s.parent_id == Some(p.id)).count() as u64;
    input += parents.iter().map(|p| PROMPT_TOKENS + children(p) * 220).sum::<u64>();
    input += PROMPT_TOKENS + sections.iter().filter(|s| s.parent_id.is_none()).count() as u64 * 180;
    let requests = (leaves.len() + parents.len() + 1) as u64;
    let output = requests * OUTPUT_TOKENS;
    let model = llm.claude_model().map(str::to_string);
    let cost = model.as_deref().and_then(|m| Usage { input, output, ..Default::default() }.cost(m));
    Ok(Estimate { sections: leaves.len() + parents.len(), model, input_tokens: input, output_tokens: output, cost })
}

/// Fixed prompt overhead per request (system prompt, instructions, schema), and typical output
/// at low effort including thinking. Used only for the estimate.
const PROMPT_TOKENS: u64 = 600;
const OUTPUT_TOKENS: u64 = 900;

/// Sections still to do: leaves, and parents deepest first (children before parents).
fn plan(sections: &[Section], redo: bool) -> (Vec<Section>, Vec<Section>) {
    let mut children: HashMap<i64, usize> = HashMap::new();
    for s in sections {
        if let Some(p) = s.parent_id {
            *children.entry(p).or_default() += 1;
        }
    }
    let todo = |s: &&Section| redo || s.summary.is_none();
    let leaves = sections.iter().filter(|s| !children.contains_key(&s.id)).filter(todo).cloned().collect();
    let mut parents: Vec<Section> = sections.iter().filter(|s| children.contains_key(&s.id)).filter(todo).cloned().collect();
    parents.sort_by_key(|s| -s.depth);
    (leaves, parents)
}

/// Run jobs with at most PARALLEL_REQUESTS in flight, stopping early if asked to.
/// Returns false if stopped. The first error is kept in `first_err`; other jobs continue.
async fn run_all(
    state: &AppState,
    jobs: Vec<std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>>>,
    first_err: &mut Option<anyhow::Error>,
    mut on_done: impl FnMut(&str),
) -> bool {
    let sem = Arc::new(Semaphore::new(PARALLEL_REQUESTS));
    let mut set = tokio::task::JoinSet::new();
    for job in jobs {
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await?;
            job.await
        });
    }
    loop {
        if state.map_stop.load(Ordering::SeqCst) {
            set.abort_all();
            return false;
        }
        tokio::select! {
            next = set.join_next() => match next {
                None => return true,
                Some(Ok(Ok(title))) => on_done(&title),
                Some(Ok(Err(e))) => { first_err.get_or_insert(e); }
                Some(Err(e)) if e.is_cancelled() => {}
                Some(Err(e)) => { first_err.get_or_insert(e.into()); }
            },
            _ = tokio::time::sleep(std::time::Duration::from_millis(150)) => {}
        }
    }
}

pub async fn build_map(state: Arc<AppState>, doc_id: String, redo: bool) -> Result<Outcome> {
    state.map_stop.store(false, Ordering::SeqCst);
    let meter: Meter = Arc::new(Mutex::new(Usage::default()));
    let claude = state.map_llm()?.with_meter(meter.clone());
    let model = claude.claude_model().map(str::to_string);
    let (doc, sections) = state.with_db(|c| {
        let doc = db::get_document(c, &doc_id)?.ok_or_else(|| anyhow!("unknown document"))?;
        Ok((doc, db::sections(c, &doc_id)?))
    })?;
    let (leaves, parents) = plan(&sections, redo);

    let total = leaves.len() + parents.len() + 1;
    let done = Arc::new(AtomicUsize::new(0));
    let progress = |current: &str| {
        let usage = model.as_ref().map(|_| *meter.lock().unwrap());
        let _ = state.app.emit(
            "map-progress",
            Progress {
                doc_id: doc_id.clone(),
                done: done.load(Ordering::SeqCst),
                total,
                current: current.to_string(),
                cost: usage.zip(model.as_deref()).and_then(|(u, m)| u.cost(m)),
                usage,
            },
        );
    };
    let outcome = |stopped: bool| {
        let usage = model.as_ref().map(|_| *meter.lock().unwrap());
        Outcome {
            stopped,
            sections: done.load(Ordering::SeqCst),
            cost: usage.zip(model.as_deref()).and_then(|(u, m)| u.cost(m)),
            usage,
            model: model.clone(),
        }
    };
    progress("Starting");
    let mut first_err = None;

    // 1. Leaves, from their text.
    let jobs = leaves
        .into_iter()
        .map(|s| {
            let (state, claude, done) = (state.clone(), claude.clone(), done.clone());
            let (doc_id, title) = (doc_id.clone(), doc.title.clone());
            Box::pin(async move {
                let text = state.with_db(|c| db::page_text(c, &doc_id, s.start_page, s.end_page))?;
                let a = analyze_leaf(&claude, &title, &s, &text).await?;
                state.with_db(|c| db::save_section_analysis(c, &doc_id, s.id, &a.summary, &a.concepts, &a.questions))?;
                done.fetch_add(1, Ordering::SeqCst);
                state.notify_changed(&doc_id);
                Ok(s.title)
            }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>>
        })
        .collect();
    if !run_all(&state, jobs, &mut first_err, |t| progress(t)).await {
        progress("Stopped");
        return Ok(outcome(true));
    }
    if let Some(e) = first_err {
        return Err(e.context("Some sections couldn't be mapped; run the map again to finish them"));
    }

    // 2. Parents, from their children's summaries; one depth level at a time, each level in parallel.
    let mut levels: Vec<Vec<Section>> = Vec::new();
    for p in parents {
        match levels.last_mut() {
            Some(l) if l[0].depth == p.depth => l.push(p),
            _ => levels.push(vec![p]),
        }
    }
    for level in levels {
        let jobs = level
            .into_iter()
            .map(|p| {
                let (state, claude, done) = (state.clone(), claude.clone(), done.clone());
                let (doc_id, title) = (doc_id.clone(), doc.title.clone());
                Box::pin(async move {
                    let kids: Vec<Section> = state.with_db(|c| {
                        Ok(db::sections(c, &doc_id)?.into_iter().filter(|s| s.parent_id == Some(p.id)).collect())
                    })?;
                    let a = analyze_parent(&claude, &title, &p, &kids).await?;
                    state.with_db(|c| db::save_section_analysis(c, &doc_id, p.id, &a.summary, &a.concepts, &a.questions))?;
                    done.fetch_add(1, Ordering::SeqCst);
                    state.notify_changed(&doc_id);
                    Ok(p.title)
                }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>>
            })
            .collect();
        if !run_all(&state, jobs, &mut first_err, |t| progress(t)).await {
            progress("Stopped");
            return Ok(outcome(true));
        }
        if let Some(e) = first_err {
            return Err(e.context("Some sections couldn't be mapped; run the map again to finish them"));
        }
    }

    // 3. Whole-document overview.
    if state.map_stop.load(Ordering::SeqCst) {
        progress("Stopped");
        return Ok(outcome(true));
    }
    progress("Document overview");
    let top: Vec<Section> = state.with_db(|c| {
        Ok(db::sections(c, &doc_id)?.into_iter().filter(|s| s.parent_id.is_none()).collect())
    })?;
    let overview = overview(&claude, &doc.title, doc.page_count, &top).await?;
    state.with_db(|c| db::set_overview(c, &doc_id, &overview))?;
    done.fetch_add(1, Ordering::SeqCst);
    state.notify_changed(&doc_id);
    progress("Done");
    Ok(outcome(false))
}

async fn analyze_leaf(claude: &Llm, doc_title: &str, s: &Section, text: &str) -> Result<Analysis> {
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

async fn analyze_parent(claude: &Llm, doc_title: &str, p: &Section, kids: &[Section]) -> Result<Analysis> {
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

async fn overview(claude: &Llm, title: &str, pages: i64, top: &[Section]) -> Result<String> {
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

async fn run(claude: &Llm, prompt: String) -> Result<Analysis> {
    let text = claude
        .complete(&Request {
            system: vec![SYSTEM.into()],
            messages: vec![json!({"role": "user", "content": prompt})],
            max_tokens: 16000,
            effort: "low",
            schema: Some(analysis_schema()),
        })
        .await?;
    claude.parse_json(&text)
}
