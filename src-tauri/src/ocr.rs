//! Local text recognition with Qianfan-OCR (baidu/Qianfan-OCR, GGUF build from ggml-org),
//! served by llama.cpp's `llama-server` as a child process that the app starts on demand
//! and stops after a few idle minutes, so the ~5 GB model isn't kept in memory.

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
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
    Starting {
        detail: String,
        progress: Option<f64>,
    },
    Ready,
    Error {
        message: String,
    },
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
    /// Where "Install" puts llama.cpp: <app data>/llama.cpp
    install_dir: PathBuf,
    installing: tokio::sync::Mutex<()>,
    http: reqwest::Client,
    server: tokio::sync::Mutex<Option<Server>>,
    status: Mutex<Status>,
    last_used: Mutex<Instant>,
    /// Settings' "Load when Anagnost opens": then the model also stays loaded instead of
    /// unloading after a few idle minutes.
    keep_loaded: std::sync::atomic::AtomicBool,
    log: Arc<Mutex<VecDeque<String>>>,
}

impl Ocr {
    pub fn new(app: AppHandle, data_dir: PathBuf) -> Arc<Self> {
        let ocr = Arc::new(Self {
            app,
            install_dir: data_dir.join("llama.cpp"),
            installing: tokio::sync::Mutex::new(()),
            http: reqwest::Client::new(),
            server: tokio::sync::Mutex::new(None),
            status: Mutex::new(Status::Stopped),
            last_used: Mutex::new(Instant::now()),
            keep_loaded: std::sync::atomic::AtomicBool::new(false),
            log: Arc::new(Mutex::new(VecDeque::new())),
        });
        let weak = Arc::downgrade(&ocr);
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let Some(ocr) = weak.upgrade() else { break };
                let idle = ocr.last_used.lock().unwrap().elapsed() > IDLE_STOP
                    && !ocr.keep_loaded.load(std::sync::atomic::Ordering::Relaxed);
                if idle && *ocr.status.lock().unwrap() == Status::Ready {
                    ocr.stop().await;
                }
            }
        });
        ocr
    }

    pub fn info(&self, configured: Option<&str>) -> Info {
        Info {
            binary: self.binary(configured).map(|p| p.display().to_string()),
            model: MODEL,
            downloaded: model_downloaded(),
            status: self.status.lock().unwrap().clone(),
        }
    }

    pub fn set_keep_loaded(&self, on: bool) {
        self.keep_loaded.store(on, std::sync::atomic::Ordering::Relaxed);
        *self.last_used.lock().unwrap() = Instant::now();
    }

    /// Load the model now if everything is already on this Mac; never starts a download.
    pub async fn autoload(&self, configured: Option<&str>) {
        if self.binary(configured).is_some() && model_downloaded() {
            let _ = self.ensure_running(configured).await;
            *self.last_used.lock().unwrap() = Instant::now();
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

        let binary = self.binary(configured).ok_or_else(|| {
            anyhow!("llama-server wasn't found. Install it in Settings → Text recognition, or set its path there.")
        })?;
        let port = free_port()?;
        let downloading = !model_downloaded();
        self.set_status(Status::Starting {
            detail: if downloading {
                "Downloading Qianfan-OCR".into()
            } else {
                "Loading Qianfan-OCR".into()
            },
            progress: downloading.then_some(0.0),
        });

        let mut child = Command::new(&binary)
            .args([
                "-hf",
                MODEL,
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
            ])
            .args(["-c", "16384", "-np", "1", "--jinja", "--no-webui"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("could not start {}", binary.display()))?;

        self.log.lock().unwrap().clear();
        for pipe in [
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        ]
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
                let tail = self
                    .log
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .take(6)
                    .cloned()
                    .collect::<Vec<_>>();
                let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
                let message = format!("llama-server exited ({code}).\n{tail}");
                self.set_status(Status::Error {
                    message: message.clone(),
                });
                bail!(message);
            }
            match self
                .http
                .get(&health)
                .timeout(Duration::from_secs(2))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => break,
                Ok(_) => self.set_status(Status::Starting {
                    detail: "Loading Qianfan-OCR".into(),
                    progress: None,
                }),
                Err(_) if downloading && !model_downloaded() => {
                    let p = (download_bytes() as f64 / MODEL_BYTES as f64).min(0.99);
                    self.set_status(Status::Starting {
                        detail: "Downloading Qianfan-OCR".into(),
                        progress: Some(p),
                    });
                }
                Err(_) => {}
            }
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
        *guard = Some(Server { child, port });
        self.set_status(Status::Ready);
        Ok(port)
    }

    /// A configured path wins; otherwise the copy Settings installed, then Homebrew and PATH.
    fn binary(&self, configured: Option<&str>) -> Option<PathBuf> {
        if configured.is_some_and(|p| !p.trim().is_empty()) {
            return find_binary(configured);
        }
        let installed = self.install_dir.join("llama-server");
        if installed.is_file() {
            return Some(installed);
        }
        find_binary(None)
    }

    /// Settings' "Install and load": get llama.cpp if it's missing, then download (first time)
    /// and load the model. Progress goes out as ocr-status events.
    pub async fn setup(&self, configured: Option<&str>) -> Result<()> {
        if self.binary(configured).is_none() {
            if configured.is_some_and(|p| !p.trim().is_empty()) {
                bail!(
                    "The llama-server path in Settings doesn't point to a file. Clear it to install llama.cpp automatically."
                );
            }
            if let Err(e) = self.install().await {
                let message = format!("Could not install llama.cpp: {e:#}");
                self.set_status(Status::Error {
                    message: message.clone(),
                });
                bail!(message);
            }
        }
        self.ensure_running(configured).await?;
        *self.last_used.lock().unwrap() = Instant::now();
        Ok(())
    }

    /// Download the newest official macOS build of llama.cpp from GitHub into install_dir.
    async fn install(&self) -> Result<()> {
        let _guard = self.installing.lock().await;
        if self.install_dir.join("llama-server").is_file() {
            return Ok(());
        }
        let detail = "Installing llama.cpp";
        self.set_status(Status::Starting {
            detail: detail.into(),
            progress: None,
        });
        download_llama(&self.http, &self.install_dir, |p| {
            self.set_status(Status::Starting {
                detail: detail.into(),
                progress: Some(p),
            })
        })
        .await
    }

    /// Recognize one page image (base64 PNG) and return its Markdown.
    pub async fn recognize(&self, configured: Option<&str>, png_base64: &str) -> Result<String> {
        if png_base64.is_empty()
            || !png_base64
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
        {
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
        let v: Value = resp
            .json()
            .await
            .context("OCR server sent an invalid response")?;
        if !status.is_success() {
            bail!(
                "OCR failed ({status}): {}",
                v["error"]["message"].as_str().unwrap_or("unknown error")
            );
        }
        *self.last_used.lock().unwrap() = Instant::now();
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default();
        Ok(clean(text))
    }
}

/// Download the newest official macOS build of llama.cpp (ggml-org/llama.cpp releases on
/// GitHub, about 17 MB) and unpack it so that `install_dir/llama-server` exists.
async fn download_llama(
    http: &reqwest::Client,
    install_dir: &Path,
    mut progress: impl FnMut(f64),
) -> Result<()> {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let suffix = format!("-bin-macos-{arch}.tar.gz");
    let releases: Value = http
        .get("https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=10")
        .header("User-Agent", "anagnost")
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .context("could not reach GitHub")?
        .error_for_status()
        .context("GitHub refused the request")?
        .json()
        .await?;
    let url = releases
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|r| r["assets"].as_array().into_iter().flatten())
        .find_map(|a| {
            let name = a["name"].as_str()?;
            name.ends_with(&suffix)
                .then(|| a["browser_download_url"].as_str())
                .flatten()
        })
        .ok_or_else(|| anyhow!("no macOS {arch} build in the recent llama.cpp releases"))?
        .to_string();
    if !url.starts_with("https://github.com/ggml-org/llama.cpp/releases/download/") {
        bail!("unexpected download address {url}");
    }

    let parent = install_dir.parent().context("bad install folder")?;
    std::fs::create_dir_all(parent)?;
    let staging = parent.join("llama.cpp.partial");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let archive = staging.join("llama.tar.gz");

    let mut resp = http
        .get(&url)
        .header("User-Agent", "anagnost")
        .timeout(Duration::from_secs(600))
        .send()
        .await?
        .error_for_status()?;
    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&archive).await?;
    let mut done = 0u64;
    while let Some(chunk) = resp.chunk().await? {
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
        done += chunk.len() as u64;
        if total > 0 {
            progress(done as f64 / total as f64);
        }
    }
    tokio::io::AsyncWriteExt::flush(&mut file).await?;
    drop(file);

    let out = Command::new("/usr/bin/tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&staging)
        .output()
        .await?;
    if !out.status.success() {
        bail!(
            "could not unpack the download: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    // The archive holds one folder (llama-bNNNN/) with the binaries and their libraries.
    let folder = std::fs::read_dir(&staging)?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.join("llama-server").is_file())
        .ok_or_else(|| anyhow!("llama-server isn't in the download"))?;
    let check = Command::new(folder.join("llama-server"))
        .arg("--version")
        .output()
        .await?;
    if !check.status.success() {
        bail!(
            "the downloaded llama-server doesn't run: {}",
            String::from_utf8_lossy(&check.stderr).trim()
        );
    }
    let _ = std::fs::remove_dir_all(install_dir);
    std::fs::rename(&folder, install_dir)?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
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
    dirs.into_iter()
        .map(|d| d.join("llama-server"))
        .find(|p| p.is_file())
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
    let Some(snapshots) = hub_dir().map(|h| h.join(REPO_DIR).join("snapshots")) else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(snapshots) else {
        return false;
    };
    entries.flatten().any(|snap| {
        MODEL_FILES
            .iter()
            .all(|f| std::fs::metadata(snap.path().join(f)).is_ok())
    })
}

/// Bytes written so far into the repo's blob store, including partial downloads.
fn download_bytes() -> u64 {
    let Some(blobs) = hub_dir().map(|h| h.join(REPO_DIR).join("blobs")) else {
        return 0;
    };
    std::fs::read_dir(blobs)
        .map(|it| {
            it.flatten()
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

fn free_port() -> Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

#[cfg(test)]
mod tests {
    use super::{clean, download_llama};

    /// Downloads about 17 MB from GitHub: cargo test -- --ignored installs_llama
    #[tokio::test]
    #[ignore]
    async fn installs_llama() {
        let dir = std::env::temp_dir().join(format!("anagnost-llama-{}", std::process::id()));
        let target = dir.join("llama.cpp");
        let mut last = 0.0;
        download_llama(&reqwest::Client::new(), &target, |p| last = p)
            .await
            .unwrap();
        assert!(target.join("llama-server").is_file());
        assert!(last > 0.99);
        assert!(!dir.join("llama.cpp.partial").exists());
        let out = std::process::Command::new(target.join("llama-server"))
            .arg("--version")
            .output()
            .unwrap();
        assert!(out.status.success());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn strips_layout_thinking() {
        assert_eq!(clean("<think>[box]</think>\n\n# Title\n"), "# Title");
        assert_eq!(clean("  plain  "), "plain");
    }
}
