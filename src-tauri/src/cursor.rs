//! Cursor as a model provider ("bring your own key"): runs the `cursor-agent` CLI headless,
//! with the user's Cursor API key, and reads its stream-json output.
//!
//! The agent is kept on a short leash: read-only "ask" mode, an empty throwaway workspace,
//! a workspace config that denies shell commands and file writes, and no `--force`, so it
//! can't run anything without an approval that a headless run never gives.

use crate::claude::Request;
use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

/// Prompts longer than this go through a file in the workspace instead of argv
/// (macOS caps the total argument size at 1 MB).
const MAX_ARG_PROMPT: usize = 120_000;
const TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Clone)]
pub struct Cursor {
    binary: PathBuf,
    api_key: Option<String>,
    model: Option<String>,
}

impl Cursor {
    pub fn new(binary: PathBuf, api_key: Option<String>, model: Option<String>) -> Self {
        Self { binary, api_key, model: model.filter(|m| !m.trim().is_empty()) }
    }

    pub async fn complete(&self, req: &Request) -> Result<String> {
        self.run(req, |_| {}).await
    }

    pub async fn stream(&self, req: &Request, on_text: impl FnMut(&str)) -> Result<String> {
        self.run(req, on_text).await
    }

    async fn run(&self, req: &Request, mut on_text: impl FnMut(&str)) -> Result<String> {
        let ws = Workspace::new()?;
        let prompt = build_prompt(req);
        let arg = if prompt.len() > MAX_ARG_PROMPT {
            std::fs::write(ws.path.join("context.md"), &prompt)?;
            "Read the file context.md in this workspace. It holds your instructions and a conversation. \
             Follow the instructions and reply to the last user message. Reply with the answer only."
                .to_string()
        } else {
            prompt
        };

        let mut cmd = Command::new(&self.binary);
        cmd.args(["-p", "--output-format", "stream-json", "--stream-partial-output", "--mode", "ask", "--trust"])
            .arg("--workspace")
            .arg(&ws.path);
        if let Some(m) = &self.model {
            cmd.args(["--model", m]);
        }
        // The key goes through the environment, so it never appears in the process list.
        if let Some(k) = &self.api_key {
            cmd.env("CURSOR_API_KEY", k);
        }
        cmd.arg(arg)
            .current_dir(&ws.path)
            .env("NO_OPEN_BROWSER", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().with_context(|| format!("could not start {}", self.binary.display()))?;

        let mut stderr = child.stderr.take().unwrap();
        let err_task = tokio::spawn(async move {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s).await;
            s
        });

        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut streamed = String::new();
        let mut result: Option<String> = None;
        let mut failure: Option<String> = None;
        let read = async {
            while let Some(line) = lines.next_line().await? {
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                match v["type"].as_str() {
                    // With --stream-partial-output, each assistant event carries one text delta.
                    Some("assistant") => {
                        for block in v["message"]["content"].as_array().into_iter().flatten() {
                            if block["type"] == "text"
                                && let Some(t) = block["text"].as_str()
                            {
                                streamed.push_str(t);
                                on_text(t);
                            }
                        }
                    }
                    Some("result") => {
                        if v["is_error"].as_bool() == Some(true) || v["subtype"] != "success" {
                            failure = Some(v["result"].as_str().unwrap_or("Cursor reported an error").to_string());
                        } else {
                            result = v["result"].as_str().map(str::to_string);
                        }
                    }
                    _ => {}
                }
            }
            anyhow::Ok(())
        };
        match tokio::time::timeout(TIMEOUT, read).await {
            Ok(r) => r?,
            Err(_) => {
                let _ = child.kill().await;
                bail!("Cursor didn't answer within {} minutes.", TIMEOUT.as_secs() / 60);
            }
        }
        let status = child.wait().await?;
        let stderr = strip_ansi(&err_task.await.unwrap_or_default());

        if let Some(f) = failure {
            bail!("Cursor: {f}");
        }
        if let Some(r) = result {
            return Ok(r);
        }
        if !status.success() || streamed.is_empty() {
            bail!(friendly_error(stderr.trim(), status.code()));
        }
        Ok(streamed)
    }
}

/// Flatten the system prompt and the conversation into one prompt for the agent.
fn build_prompt(req: &Request) -> String {
    let mut p = String::from("<instructions>\n");
    for s in &req.system {
        p.push_str(s);
        p.push_str("\n\n");
    }
    p.push_str(
        "You are answering inside a PDF reading app, not working on code. Everything you need is in \
         this prompt: don't use tools, don't read or write files, don't run commands.\n</instructions>\n\n",
    );
    let n = req.messages.len();
    for (i, m) in req.messages.iter().enumerate() {
        let role = m["role"].as_str().unwrap_or("user");
        let text = m["content"].as_str().unwrap_or_default();
        if n == 1 {
            p.push_str(text);
        } else {
            p.push_str(&format!("<{role}>\n{text}\n</{role}>\n\n"));
            if i + 1 == n {
                p.push_str("Reply to the last user message as the assistant.");
            }
        }
    }
    if let Some(schema) = &req.schema {
        p.push_str(&format!(
            "\n\nRespond with only a JSON object that matches this JSON Schema. No prose, no code fences.\n{schema}"
        ));
    }
    p
}

fn friendly_error(stderr: &str, code: Option<i32>) -> String {
    let lower = stderr.to_lowercase();
    if lower.contains("api key is invalid") {
        return "Cursor rejected the API key. Check it in Settings, or create a new one in the Cursor dashboard.".into();
    }
    if lower.contains("not logged in") || lower.contains("authenticate") {
        return "Cursor isn't signed in. Add a Cursor API key in Settings.".into();
    }
    if stderr.is_empty() {
        return format!("cursor-agent exited without an answer (code {}).", code.map_or("?".into(), |c| c.to_string()));
    }
    let tail: Vec<&str> = stderr.lines().rev().take(4).collect();
    format!("Cursor: {}", tail.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Extract the JSON object from a reply that may wrap it in prose or a code fence.
pub fn extract_json(text: &str) -> Result<&str> {
    let start = text.find('{').ok_or_else(|| anyhow!("no JSON object in the reply"))?;
    let end = text.rfind('}').ok_or_else(|| anyhow!("no JSON object in the reply"))?;
    if end < start {
        bail!("no JSON object in the reply");
    }
    Ok(&text[start..=end])
}

/// An empty, private working directory for one run, removed afterwards.
struct Workspace {
    path: PathBuf,
}

impl Workspace {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("pdf-tutor-cursor-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(path.join(".cursor"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
        std::fs::write(
            path.join(".cursor/cli.json"),
            r#"{"permissions":{"allow":[],"deny":["Shell(*)","Write(**)"]}}"#,
        )?;
        Ok(Self { path })
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Find cursor-agent: the configured path, the Cursor installer's location, or PATH.
pub fn find_binary(configured: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = configured.filter(|p| !p.trim().is_empty()) {
        let p = PathBuf::from(p.trim());
        return p.is_file().then_some(p);
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join(".local/bin"));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.into_iter().map(|d| d.join("cursor-agent")).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(messages: Vec<Value>, schema: Option<Value>) -> Request {
        Request { system: vec!["SYS".into(), "CONTEXT".into()], messages, max_tokens: 1, effort: "low", schema }
    }

    #[test]
    fn single_prompt_is_inlined_with_instructions() {
        let p = build_prompt(&req(vec![json!({"role": "user", "content": "Summarize."})], Some(json!({"type": "object"}))));
        assert!(p.starts_with("<instructions>\nSYS\n\nCONTEXT"));
        assert!(p.contains("don't run commands"));
        assert!(p.contains("Summarize."));
        assert!(p.trim_end().ends_with(r#"{"type":"object"}"#));
    }

    #[test]
    fn conversation_is_tagged_by_role() {
        let p = build_prompt(&req(
            vec![
                json!({"role": "user", "content": "Q1"}),
                json!({"role": "assistant", "content": "A1"}),
                json!({"role": "user", "content": "Q2"}),
            ],
            None,
        ));
        assert!(p.contains("<user>\nQ1\n</user>\n\n<assistant>\nA1\n</assistant>\n\n<user>\nQ2\n</user>"));
        assert!(p.ends_with("Reply to the last user message as the assistant."));
    }

    #[test]
    fn json_is_extracted_from_fences_and_prose() {
        assert_eq!(extract_json("```json\n{\"a\": {\"b\": 1}}\n```").unwrap(), "{\"a\": {\"b\": 1}}");
        assert_eq!(extract_json("Here you go: {\"a\":1} done").unwrap(), "{\"a\":1}");
        assert!(extract_json("no json").is_err());
    }

    #[test]
    fn ansi_codes_are_removed() {
        assert_eq!(strip_ansi("\u{1b}[33m⚠ Warning: bad key\u{1b}[0m"), "⚠ Warning: bad key");
    }

    #[test]
    fn workspace_denies_shell_and_writes_and_is_removed() {
        let ws = Workspace::new().unwrap();
        let path = ws.path.clone();
        let cfg = std::fs::read_to_string(path.join(".cursor/cli.json")).unwrap();
        assert!(cfg.contains("Shell(*)") && cfg.contains("Write(**)"));
        drop(ws);
        assert!(!path.exists());
    }

    /// Runs the real cursor-agent with a bad key (no usage is billed). `cargo test -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn invalid_key_gives_a_clear_error() {
        let Some(bin) = find_binary(None) else { return };
        let c = Cursor::new(bin, Some("invalid-test-key".into()), None);
        let err = c.complete(&req(vec![json!({"role": "user", "content": "Say hi"})], None)).await.unwrap_err();
        assert!(err.to_string().contains("rejected the API key"), "{err}");
    }
}
