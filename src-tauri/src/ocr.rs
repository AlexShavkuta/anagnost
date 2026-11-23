//! Local text recognition with Qianfan-OCR (baidu/Qianfan-OCR, GGUF build from ggml-org),
//! served by llama.cpp's `llama-server` as a child process that the app starts on demand
//! and stops after a few idle minutes, so the ~5 GB model isn't kept in memory.

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// Hugging Face repo and quantization passed to `llama-server -hf`.
pub const MODEL: &str = "ggml-org/Qianfan-OCR-GGUF:Q8_0";
const REPO_DIR: &str = "models--ggml-org--Qianfan-OCR-GGUF";
const MODEL_FILES: [&str; 2] = ["Qianfan-OCR-Q8_0.gguf", "mmproj-Qianfan-OCR-Q8_0.gguf"];
/// Combined size of the two files, for download progress.
const MODEL_BYTES: u64 = 4_703_000_000 + 347_000_000;
const PROMPT: &str = "Parse this document to Markdown.";
const IDLE_STOP: Duration = Duration::from_secs(5 * 60);

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Stopped,
    /// `progress` is 0..1 while the model downloads, None while it loads.
    Starting { detail: String, progress: Option<f64> },
    Ready,
    Error { message: String },
}

#[derive(Serialize)]
pub struct Info {
    pub binary: Option<String>,
    pub model: &'static str,
    pub downloaded: bool,
    pub status: Status,
}

struct Server {
    child: Child,
    port: u16,
}

pub struct Ocr {
    app: AppHandle,
    http: reqwest::Client,
    server: tokio::sync::Mutex<Option<Server>>,
    status: Mutex<Status>,
    last_used: Mutex<Instant>,
    log: Arc<Mutex<VecDeque<String>>>,
}

impl Ocr {
    pub fn new(app: AppHandle) -> Arc<Self> {
        let ocr = Arc::new(Self {
            app,
            http: reqwest::Client::new(),
            server: tokio::sync::Mutex::new(None),
            status: Mutex::new(Status::Stopped),
            last_used: Mutex::new(Instant::now()),
            log: Arc::new(Mutex::new(VecDeque::new())),
        });
        let weak = Arc::downgrade(&ocr);
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let Some(ocr) = weak.upgrade() else { break };
                let idle = ocr.last_used.lock().unwrap().elapsed() > IDLE_STOP;
                if idle && *ocr.status.lock().unwrap() == Status::Ready {
                    ocr.stop().await;
                }
            }
        });
        ocr
    }

    pub fn info(&self, configured: Option<&str>) -> Info {
        Info {
            binary: find_binary(configured).map(|p| p.display().to_string()),
            model: MODEL,
            downloaded: model_downloaded(),
            status: self.status.lock().unwrap().clone(),
        }
    }

    fn set_status(&self, s: Status) {
        *self.status.lock().unwrap() = s.clone();
        let _ = self.app.emit("ocr-status", s);
    }

    pub async fn stop(&self) {
        let mut guard = self.server.lock().await;
        if let Some(mut s) = guard.take() {
            let _ = s.child.kill().await;
        }
        self.set_status(Status::Stopped);
    }

    /// Kill the server without waiting; for app exit.
    pub fn kill_now(&self) {
        if let Ok(mut guard) = self.server.try_lock()
            && let Some(s) = guard.as_mut()
        {
            let _ = s.child.start_kill();
        }
    }

    /// Start llama-server if it isn't running, and wait until the model is loaded.
    /// The first start downloads the model into the Hugging Face cache.
    async fn ensure_running(&self, configured: Option<&str>) -> Result<u16> {
        let mut guard = self.server.lock().await;
        if let Some(s) = guard.as_mut() {
            if s.child.try_wait()?.is_none() {
                return Ok(s.port);
            }
            *guard = None;
        }

        let binary = find_binary(configured).ok_or_else(|| {
            anyhow!(
                "llama-server wasn't found. Install llama.cpp (brew install llama.cpp), \
                 or set its path in Settings."
            )
        })?;
        let port = free_port()?;
        let downloading = !model_downloaded();
        self.set_status(Status::Starting {
            detail: if downloading { "Downloading Qianfan-OCR".into() } else { "Loading Qianfan-OCR".into() },
            progress: downloading.then_some(0.0),
        });

        let mut child = Command::new(&binary)
            .args(["-hf", MODEL, "--host", "127.0.0.1", "--port", &port.to_string()])
            .args(["-c", "16384", "-np", "1", "--jinja", "--no-webui"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("could not start {}", binary.display()))?;

        self.log.lock().unwrap().clear();
        for pipe in [child.stdout.take().map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
                     child.stderr.take().map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)]
            .into_iter()
            .flatten()
        {
            let log = self.log.clone();
            tauri::async_runtime::spawn(async move {
                let mut lines = BufReader::new(pipe).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut log = log.lock().unwrap();
                    if log.len() >= 40 {
                        log.pop_front();
                    }
                    log.push_back(line);
                }
            });
        }

        let health = format!("http://127.0.0.1:{port}/health");
        loop {
            if let Some(code) = child.try_wait()? {
                let tail = self.log.lock().unwrap().iter().rev().take(6).cloned().collect::<Vec<_>>();
                let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
                let message = format!("llama-server exited ({code}).\n{tail}");
                self.set_status(Status::Error { message: message.clone() });
                bail!(message);
            }
            match self.http.get(&health).timeout(Duration::from_secs(2)).send().await {
                Ok(r) if r.status().is_success() => break,
                Ok(_) => self.set_status(Status::Starting { detail: "Loading Qianfan-OCR".into(), progress: None }),
                Err(_) if downloading && !model_downloaded() => {
                    let p = (download_bytes() as f64 / MODEL_BYTES as f64).min(0.99);
                    self.set_status(Status::Starting { detail: "Downloading Qianfan-OCR".into(), progress: Some(p) });
                }
                Err(_) => {}
            }
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
        *guard = Some(Server { child, port });
        self.set_status(Status::Ready);
        Ok(port)
    }

    /// Recognize one page image (base64 PNG) and return its Markdown.
    pub async fn recognize(&self, configured: Option<&str>, png_base64: &str) -> Result<String> {
        if png_base64.is_empty() || !png_base64.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b)) {
            bail!("page image is not valid base64");
        }
        let port = self.ensure_running(configured).await?;
        *self.last_used.lock().unwrap() = Instant::now();
        let image = format!("data:image/png;base64,{png_base64}");
        let body = json!({
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": image}},
                    {"type": "text", "text": PROMPT},
                ],
            }],
            "temperature": 0,
            "max_tokens": 8192,
        });
        let resp = self
            .http
            .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .json(&body)
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .context("OCR request failed")?;
        let status = resp.status();
        let v: Value = resp.json().await.context("OCR server sent an invalid response")?;
        if !status.is_success() {
            bail!("OCR failed ({status}): {}", v["error"]["message"].as_str().unwrap_or("unknown error"));
        }
        *self.last_used.lock().unwrap() = Instant::now();
        let text = v["choices"][0]["message"]["content"].as_str().unwrap_or_default();
        Ok(clean(text))
    }
}

/// Remove a layout "thinking" block if the model emitted one, and trim.
fn clean(text: &str) -> String {
    let text = match text.rfind("</think>") {
        Some(i) => &text[i + "</think>".len()..],
        None => text,
    };
    text.trim().to_string()
}

fn find_binary(configured: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = configured.filter(|p| !p.trim().is_empty()) {
        let p = PathBuf::from(p.trim());
        return p.is_file().then_some(p);
    }
    // Apps launched from Finder don't get the shell's PATH, so check Homebrew's locations too.
    let mut dirs: Vec<PathBuf> = vec!["/opt/homebrew/bin".into(), "/usr/local/bin".into()];
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/bin"));
    }
    dirs.into_iter().map(|d| d.join("llama-server")).find(|p| p.is_file())
}

fn hub_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("HF_HUB_CACHE") {
        return Some(PathBuf::from(p));
    }
    if let Some(p) = std::env::var_os("HF_HOME") {
        return Some(PathBuf::from(p).join("hub"));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/huggingface/hub"))
}

fn model_downloaded() -> bool {
    let Some(snapshots) = hub_dir().map(|h| h.join(REPO_DIR).join("snapshots")) else { return false };
    let Ok(entries) = std::fs::read_dir(snapshots) else { return false };
    entries
        .flatten()
        .any(|snap| MODEL_FILES.iter().all(|f| std::fs::metadata(snap.path().join(f)).is_ok()))
}

/// Bytes written so far into the repo's blob store, including partial downloads.
fn download_bytes() -> u64 {
    let Some(blobs) = hub_dir().map(|h| h.join(REPO_DIR).join("blobs")) else { return 0 };
    std::fs::read_dir(blobs)
        .map(|it| it.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
        .unwrap_or(0)
}

fn free_port() -> Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn strips_layout_thinking() {
        assert_eq!(clean("<think>[box]</think>\n\n# Title\n"), "# Title");
        assert_eq!(clean("  plain  "), "plain");
    }
}
