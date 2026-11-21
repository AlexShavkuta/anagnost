//! SQLite storage: documents, per-page text, the section tree (the "map"), and notes.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS documents (
            id          TEXT PRIMARY KEY,           -- sha256 of file bytes
            path        TEXT NOT NULL,
            title       TEXT NOT NULL,
            page_count  INTEGER NOT NULL,
            last_page   INTEGER NOT NULL DEFAULT 1,
            overview    TEXT,
            opened_at   INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS pages (
            doc_id  TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
            page    INTEGER NOT NULL,
            text    TEXT NOT NULL,
            PRIMARY KEY (doc_id, page)
        );

        CREATE TABLE IF NOT EXISTS sections (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            doc_id      TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
            parent_id   INTEGER REFERENCES sections(id) ON DELETE CASCADE,
            ord         INTEGER NOT NULL,
            depth       INTEGER NOT NULL,
            title       TEXT NOT NULL,
            start_page  INTEGER NOT NULL,
            end_page    INTEGER NOT NULL,
            summary     TEXT,
            concepts    TEXT,       -- JSON array of {term, explanation}
            questions   TEXT,       -- JSON array of strings
            studied     INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS sections_doc ON sections(doc_id);

        CREATE TABLE IF NOT EXISTS notes (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            doc_id      TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
            page        INTEGER,
            text        TEXT NOT NULL,
            source      TEXT NOT NULL,   -- 'user' | 'claude-app' | 'claude-code'
            created_at  INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )?;
    Ok(conn)
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------- settings ----------

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

// ---------- documents ----------

#[derive(Serialize, Clone, Debug)]
pub struct Document {
    pub id: String,
    pub path: String,
    pub title: String,
    pub page_count: i64,
    pub last_page: i64,
    pub overview: Option<String>,
}

fn row_to_doc(r: &rusqlite::Row) -> rusqlite::Result<Document> {
    Ok(Document {
        id: r.get(0)?,
        path: r.get(1)?,
        title: r.get(2)?,
        page_count: r.get(3)?,
        last_page: r.get(4)?,
        overview: r.get(5)?,
    })
}

pub fn get_document(conn: &Connection, id: &str) -> Result<Option<Document>> {
    Ok(conn
        .query_row(
            "SELECT id, path, title, page_count, last_page, overview FROM documents WHERE id = ?1",
            [id],
            row_to_doc,
        )
        .optional()?)
}

pub fn recent_documents(conn: &Connection) -> Result<Vec<Document>> {
    let mut st = conn.prepare(
        "SELECT id, path, title, page_count, last_page, overview FROM documents
         ORDER BY opened_at DESC LIMIT 20",
    )?;
    let rows = st.query_map([], row_to_doc)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn touch_document(conn: &Connection, id: &str, path: &str) -> Result<()> {
    conn.execute(
        "UPDATE documents SET opened_at = ?2, path = ?3 WHERE id = ?1",
        params![id, now(), path],
    )?;
    Ok(())
}

pub fn set_last_page(conn: &Connection, id: &str, page: i64) -> Result<()> {
    conn.execute("UPDATE documents SET last_page = ?2 WHERE id = ?1", params![id, page])?;
    Ok(())
}

pub fn set_overview(conn: &Connection, id: &str, overview: &str) -> Result<()> {
    conn.execute("UPDATE documents SET overview = ?2 WHERE id = ?1", params![id, overview])?;
    Ok(())
}

// ---------- ingest ----------

#[derive(Deserialize, Debug, Clone)]
pub struct OutlineItem {
    pub title: String,
    /// 1-based page, if the outline entry resolved to a page.
    pub page: Option<i64>,
    #[serde(default)]
    pub items: Vec<OutlineItem>,
}

/// Insert a new document with its page texts and section tree.
/// Sections come from the PDF outline when it has one, otherwise from fixed page chunks.
pub fn ingest(
    conn: &mut Connection,
    id: &str,
    path: &str,
    title: &str,
    pages: &[String],
    outline: &[OutlineItem],
) -> Result<()> {
    let tx = conn.transaction()?;
    let page_count = pages.len() as i64;
    tx.execute(
        "INSERT INTO documents(id, path, title, page_count, opened_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, path, title, page_count, now()],
    )?;
    {
        let mut st = tx.prepare("INSERT INTO pages(doc_id, page, text) VALUES (?1, ?2, ?3)")?;
        for (i, text) in pages.iter().enumerate() {
            st.execute(params![id, (i + 1) as i64, text])?;
        }
    }

    let outline = clean_outline(outline, page_count);
    if outline.len() >= 2 {
        insert_outline(&tx, id, None, &outline, 0, 1, page_count)?;
    } else {
        insert_chunks(&tx, id, pages)?;
    }
    tx.commit()?;
    Ok(())
}

/// Drop entries without a page, and keep siblings in page order.
fn clean_outline(items: &[OutlineItem], page_count: i64) -> Vec<OutlineItem> {
    let mut out: Vec<OutlineItem> = items
        .iter()
        .filter_map(|it| {
            let page = it.page?;
            if page < 1 || page > page_count {
                return None;
            }
            Some(OutlineItem {
                title: it.title.trim().to_string(),
                page: Some(page),
                items: clean_outline(&it.items, page_count),
            })
        })
        .collect();
    out.sort_by_key(|it| it.page);
    out
}

fn insert_outline(
    tx: &rusqlite::Transaction,
    doc_id: &str,
    parent: Option<i64>,
    items: &[OutlineItem],
    depth: i64,
    range_start: i64,
    range_end: i64,
) -> Result<()> {
    for (i, it) in items.iter().enumerate() {
        let start = it.page.unwrap_or(range_start).max(range_start);
        let end = match items.get(i + 1).and_then(|n| n.page) {
            Some(next) => (next - 1).max(start),
            None => range_end,
        }
        .min(range_end);
        tx.execute(
            "INSERT INTO sections(doc_id, parent_id, ord, depth, title, start_page, end_page)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![doc_id, parent, i as i64, depth, it.title, start, end],
        )?;
        let sid = tx.last_insert_rowid();
        // Outline depth beyond 3 levels is too fine-grained to be useful as a study map.
        if !it.items.is_empty() && depth < 2 {
            insert_outline(tx, doc_id, Some(sid), &it.items, depth + 1, start, end)?;
        }
    }
    Ok(())
}

/// No usable outline: group pages into chunks of roughly CHUNK_WORDS words.
fn insert_chunks(tx: &rusqlite::Transaction, doc_id: &str, pages: &[String]) -> Result<()> {
    const CHUNK_WORDS: usize = 4000;
    let mut start = 1usize;
    let mut words = 0usize;
    let mut ord = 0i64;
    for (i, text) in pages.iter().enumerate() {
        words += text.split_whitespace().count();
        let page = i + 1;
        if words >= CHUNK_WORDS || page == pages.len() {
            let title = if start == page {
                format!("Page {start}")
            } else {
                format!("Pages {start}–{page}")
            };
            tx.execute(
                "INSERT INTO sections(doc_id, parent_id, ord, depth, title, start_page, end_page)
                 VALUES (?1, NULL, ?2, 0, ?3, ?4, ?5)",
                params![doc_id, ord, title, start as i64, page as i64],
            )?;
            ord += 1;
            start = page + 1;
            words = 0;
        }
    }
    Ok(())
}

// ---------- pages ----------

pub fn page_text(conn: &Connection, doc_id: &str, start: i64, end: i64) -> Result<String> {
    let mut st = conn.prepare(
        "SELECT page, text FROM pages WHERE doc_id = ?1 AND page BETWEEN ?2 AND ?3 ORDER BY page",
    )?;
    let mut out = String::new();
    let rows = st.query_map(params![doc_id, start, end], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (p, t) = row?;
        out.push_str(&format!("[page {p}]\n{t}\n\n"));
    }
    Ok(out)
}

#[derive(Serialize, Clone, Debug)]
pub struct SearchHit {
    pub page: i64,
    pub snippet: String,
}

pub fn search(conn: &Connection, doc_id: &str, query: &str) -> Result<Vec<SearchHit>> {
    let q = query.to_lowercase();
    let mut st = conn.prepare("SELECT page, text FROM pages WHERE doc_id = ?1 ORDER BY page")?;
    let rows = st.query_map([doc_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    let mut hits = Vec::new();
    for row in rows {
        let (page, text) = row?;
        let lower = text.to_lowercase();
        if let Some(pos) = lower.find(&q) {
            let start = floor_char(&text, pos.saturating_sub(120));
            let end = floor_char(&text, (pos + q.len() + 120).min(text.len()));
            hits.push(SearchHit { page, snippet: text[start..end].replace('\n', " ") });
            if hits.len() >= 30 {
                break;
            }
        }
    }
    Ok(hits)
}

fn floor_char(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

// ---------- sections ----------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Concept {
    pub term: String,
    pub explanation: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Section {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub ord: i64,
    pub depth: i64,
    pub title: String,
    pub start_page: i64,
    pub end_page: i64,
    pub summary: Option<String>,
    pub concepts: Vec<Concept>,
    pub questions: Vec<String>,
    pub studied: bool,
}

pub fn sections(conn: &Connection, doc_id: &str) -> Result<Vec<Section>> {
    let mut st = conn.prepare(
        "SELECT id, parent_id, ord, depth, title, start_page, end_page, summary, concepts, questions, studied
         FROM sections WHERE doc_id = ?1 ORDER BY start_page, depth, ord",
    )?;
    let rows = st
        .query_map([doc_id], |r| {
            let concepts: Option<String> = r.get(8)?;
            let questions: Option<String> = r.get(9)?;
            Ok(Section {
                id: r.get(0)?,
                parent_id: r.get(1)?,
                ord: r.get(2)?,
                depth: r.get(3)?,
                title: r.get(4)?,
                start_page: r.get(5)?,
                end_page: r.get(6)?,
                summary: r.get(7)?,
                concepts: concepts.and_then(|c| serde_json::from_str(&c).ok()).unwrap_or_default(),
                questions: questions.and_then(|q| serde_json::from_str(&q).ok()).unwrap_or_default(),
                studied: r.get::<_, i64>(10)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn section(conn: &Connection, doc_id: &str, id: i64) -> Result<Option<Section>> {
    Ok(sections(conn, doc_id)?.into_iter().find(|s| s.id == id))
}

pub fn save_section_analysis(
    conn: &Connection,
    doc_id: &str,
    id: i64,
    summary: &str,
    concepts: &[Concept],
    questions: &[String],
) -> Result<bool> {
    let n = conn.execute(
        "UPDATE sections SET summary = ?3, concepts = ?4, questions = ?5 WHERE id = ?1 AND doc_id = ?2",
        params![
            id,
            doc_id,
            summary,
            serde_json::to_string(concepts)?,
            serde_json::to_string(questions)?
        ],
    )?;
    Ok(n > 0)
}

pub fn set_studied(conn: &Connection, doc_id: &str, id: i64, studied: bool) -> Result<()> {
    conn.execute(
        "UPDATE sections SET studied = ?3 WHERE id = ?1 AND doc_id = ?2",
        params![id, doc_id, studied as i64],
    )?;
    Ok(())
}

// ---------- notes ----------

#[derive(Serialize, Clone, Debug)]
pub struct Note {
    pub id: i64,
    pub page: Option<i64>,
    pub text: String,
    pub source: String,
    pub created_at: i64,
}

pub fn add_note(conn: &Connection, doc_id: &str, page: Option<i64>, text: &str, source: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO notes(doc_id, page, text, source, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![doc_id, page, text, source, now()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn notes(conn: &Connection, doc_id: &str) -> Result<Vec<Note>> {
    let mut st = conn.prepare(
        "SELECT id, page, text, source, created_at FROM notes WHERE doc_id = ?1 ORDER BY created_at DESC",
    )?;
    let rows = st
        .query_map([doc_id], |r| {
            Ok(Note {
                id: r.get(0)?,
                page: r.get(1)?,
                text: r.get(2)?,
                source: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn delete_note(conn: &Connection, doc_id: &str, id: i64) -> Result<()> {
    conn.execute("DELETE FROM notes WHERE id = ?1 AND doc_id = ?2", params![id, doc_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let dir = std::env::temp_dir().join(format!("pdf-tutor-test-{}.db", uuid::Uuid::new_v4()));
        open(&dir).unwrap()
    }

    fn item(title: &str, page: Option<i64>, items: Vec<OutlineItem>) -> OutlineItem {
        OutlineItem { title: title.into(), page, items }
    }

    #[test]
    fn outline_becomes_nested_sections_with_page_ranges() {
        let mut c = mem();
        let pages: Vec<String> = (1..=20).map(|i| format!("text of page {i}")).collect();
        let outline = vec![
            item("Intro", Some(1), vec![]),
            item("Part A", Some(3), vec![item("A.1", Some(3), vec![]), item("A.2", Some(7), vec![])]),
            item("Broken link", None, vec![]),
            item("Part B", Some(12), vec![]),
        ];
        ingest(&mut c, "d", "/x.pdf", "Doc", &pages, &outline).unwrap();
        let s = sections(&c, "d").unwrap();
        let get = |t: &str| s.iter().find(|x| x.title == t).unwrap().clone();
        assert_eq!(s.len(), 5, "entry without a page is dropped");
        assert_eq!((get("Intro").start_page, get("Intro").end_page), (1, 2));
        assert_eq!((get("Part A").start_page, get("Part A").end_page), (3, 11));
        assert_eq!((get("A.1").start_page, get("A.1").end_page), (3, 6));
        assert_eq!((get("A.2").start_page, get("A.2").end_page), (7, 11));
        assert_eq!(get("A.2").parent_id, Some(get("Part A").id));
        assert_eq!((get("Part B").start_page, get("Part B").end_page), (12, 20));
    }

    #[test]
    fn no_outline_falls_back_to_word_chunks() {
        let mut c = mem();
        let page = "word ".repeat(1500); // 1500 words per page -> chunks of 3 pages
        let pages: Vec<String> = (0..7).map(|_| page.clone()).collect();
        ingest(&mut c, "d", "/x.pdf", "Doc", &pages, &[]).unwrap();
        let s = sections(&c, "d").unwrap();
        let ranges: Vec<_> = s.iter().map(|x| (x.start_page, x.end_page)).collect();
        assert_eq!(ranges, vec![(1, 3), (4, 6), (7, 7)]);
    }

    #[test]
    fn search_and_analysis_roundtrip() {
        let mut c = mem();
        let pages = vec!["Alpha beta".to_string(), "Gamma Délta epsilon".to_string()];
        ingest(&mut c, "d", "/x.pdf", "Doc", &pages, &[]).unwrap();
        let hits = search(&c, "d", "délta").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].page, 2);
        let id = sections(&c, "d").unwrap()[0].id;
        let concepts = vec![Concept { term: "t".into(), explanation: "e".into() }];
        assert!(save_section_analysis(&c, "d", id, "sum", &concepts, &["q?".into()]).unwrap());
        let s = section(&c, "d", id).unwrap().unwrap();
        assert_eq!(s.summary.as_deref(), Some("sum"));
        assert_eq!(s.concepts.len(), 1);
        assert!(!save_section_analysis(&c, "other-doc", id, "x", &[], &[]).unwrap());
    }
}
