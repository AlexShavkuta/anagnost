//! Updates from GitHub Releases. CI (.github/workflows/release.yml) attaches a zipped app and
//! its SHA-256 to every `v*` release. The app compares the latest release with its own version;
//! "Update" downloads the zip, checks it, and a small detached script swaps the app bundle once
//! this process has quit, then opens the new version.

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const BUNDLE_ID: &str = "com.alex.pdftutor";

const SWAP_SCRIPT: &str = r#"#!/bin/bash
# Written by Anagnost's updater. $1: new app, $2: app to replace, $3: pid to wait for,
# $4: where the new app goes (differs from $2 when the app's name changed).
set -u
for _ in $(seq 1 60); do kill -0 "$3" 2>/dev/null || break; sleep 0.5; done
rm -rf "$4.updating"
ditto "$1" "$4.updating" || exit 1
rm -rf "$2" "$4" && mv "$4.updating" "$4"
xattr -dr com.apple.quarantine "$4" 2>/dev/null
open "$4"
"#;

#[derive(Serialize, Clone)]
pub struct Release {
    pub version: String,
    /// The release's bullet points, without PR links.
    pub notes: Vec<String>,
    pub url: String,
    #[serde(skip)]
    asset: Option<(String, Option<String>)>,
    pub downloadable: bool,
}

#[derive(Serialize)]
pub struct Check {
    pub current: &'static str,
    pub latest: Option<Release>,
    pub newer: bool,
    /// Why "Update" can't install it from inside the app, if it can't.
    pub unavailable: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Idle,
    /// `progress` is 0..1 while downloading.
    Running { step: String, progress: Option<f64> },
    Error { message: String },
}

pub struct Updater {
    app: AppHandle,
    dir: PathBuf,
    http: reqwest::Client,
    status: Mutex<Status>,
    latest: Mutex<Option<Release>>,
}

impl Updater {
    pub fn new(app: AppHandle, data_dir: &Path) -> Self {
        Self {
            app,
            dir: data_dir.join("update"),
            http: reqwest::Client::new(),
            status: Mutex::new(Status::Idle),
            latest: Mutex::new(None),
        }
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    fn set_status(&self, s: Status) {
        *self.status.lock().unwrap() = s.clone();
        let _ = self.app.emit("update-status", s);
    }

    pub async fn check(&self) -> Result<Check> {
        let repo = repo().ok_or_else(|| anyhow!("This build doesn't know its GitHub repository."))?;
        // The API gives release notes, but allows only 60 anonymous requests an hour per network
        // and sometimes refuses. The web release page has no such limit, so fall back to it.
        let latest = match Self::latest_from_api(&self.http, repo).await {
            Ok(latest) => latest,
            Err(api_err) => Self::latest_from_web(&self.http, repo)
                .await
                .map_err(|web_err| anyhow!("{api_err:#}. The release page didn't answer either: {web_err:#}"))?,
        };
        let newer = latest.as_ref().is_some_and(|r| is_newer(&r.version, CURRENT));
        *self.latest.lock().unwrap() = latest.clone();
        Ok(Check { current: CURRENT, newer, unavailable: newer.then(unavailable).flatten(), latest })
    }

    async fn latest_from_api(http: &reqwest::Client, repo: &str) -> Result<Option<Release>> {
        let resp = http
            .get(format!("https://api.github.com/repos/{repo}/releases/latest"))
            .header("User-Agent", "anagnost")
            .header("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .context("Could not reach GitHub")?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None); // no releases yet
        }
        if !status.is_success() {
            let limited = resp.headers().get("x-ratelimit-remaining").is_some_and(|v| v == "0");
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            let why = if limited || status.as_u16() == 429 {
                "GitHub's hourly limit for update checks from this network was reached".to_string()
            } else {
                format!("GitHub answered {status}: {}", body["message"].as_str().unwrap_or("no details"))
            };
            bail!(why);
        }
        Ok(Some(parse_release(&resp.json().await?)))
    }

    /// github.com/<repo>/releases/latest redirects to the latest tag; the assets follow the
    /// names CI gives them. No release notes this way.
    async fn latest_from_web(http: &reqwest::Client, repo: &str) -> Result<Option<Release>> {
        let no_redirects = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build()?;
        let resp = no_redirects
            .get(format!("https://github.com/{repo}/releases/latest"))
            .header("User-Agent", "anagnost")
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .context("Could not reach github.com")?;
        let location = resp.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        let Some(tag) = location.split("/releases/tag/").nth(1).filter(|t| !t.is_empty()) else {
            if resp.status() == reqwest::StatusCode::NOT_FOUND || location.ends_with("/releases") {
                return Ok(None); // no releases yet
            }
            bail!("github.com answered {}", resp.status());
        };
        let version = tag.trim_start_matches('v').to_string();
        let zip = format!("https://github.com/{repo}/releases/download/{tag}/Anagnost-{version}-macos-universal.zip");
        let exists = http
            .head(&zip)
            .header("User-Agent", "anagnost")
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        let asset = exists.then(|| (zip.clone(), Some(format!("{zip}.sha256"))));
        Ok(Some(Release { version, notes: Vec::new(), url: location, downloadable: asset.is_some(), asset }))
    }

    /// Download and verify the latest release, then hand over to the swap script and quit.
    pub fn start(&self) -> Result<()> {
        if matches!(self.status(), Status::Running { .. }) {
            return Ok(());
        }
        if let Some(why) = unavailable() {
            bail!(why);
        }
        let release = self.latest.lock().unwrap().clone().ok_or_else(|| anyhow!("Check for updates first."))?;
        let (zip_url, sha_url) = release.asset.clone().ok_or_else(|| anyhow!("This release has no macOS download."))?;
        self.set_status(Status::Running { step: "Downloading".into(), progress: Some(0.0) });
        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            let state = tauri::Manager::state::<std::sync::Arc<crate::state::AppState>>(&app);
            let me = &state.updater;
            match me.install(&zip_url, sha_url.as_deref()).await {
                Ok(()) => {
                    me.set_status(Status::Running { step: "Restarting".into(), progress: None });
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    app.exit(0);
                }
                Err(e) => me.set_status(Status::Error { message: format!("{e:#}") }),
            }
        });
        Ok(())
    }

    async fn install(&self, zip_url: &str, sha_url: Option<&str>) -> Result<()> {
        let bundle = bundle_path().ok_or_else(|| anyhow!("Anagnost isn't running from an installed app."))?;
        let _ = std::fs::remove_dir_all(&self.dir);
        std::fs::create_dir_all(&self.dir)?;
        let zip = self.dir.join("Anagnost.zip");

        let mut resp = self.http.get(zip_url).header("User-Agent", "anagnost").timeout(Duration::from_secs(900)).send().await?.error_for_status()?;
        let total = resp.content_length().unwrap_or(0);
        let mut file = tokio::fs::File::create(&zip).await?;
        let mut hash = Sha256::new();
        let mut done = 0u64;
        while let Some(chunk) = resp.chunk().await? {
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
            hash.update(&chunk);
            done += chunk.len() as u64;
            if total > 0 {
                self.set_status(Status::Running { step: "Downloading".into(), progress: Some(done as f64 / total as f64) });
            }
        }
        tokio::io::AsyncWriteExt::flush(&mut file).await?;
        drop(file);

        self.set_status(Status::Running { step: "Installing".into(), progress: None });
        if let Some(sha_url) = sha_url {
            let expected = self.http.get(sha_url).header("User-Agent", "anagnost").send().await?.error_for_status()?.text().await?;
            let expected = expected.split_whitespace().next().unwrap_or_default().to_lowercase();
            if hex::encode(hash.finalize()) != expected {
                bail!("The download is damaged (checksum mismatch). Try again.");
            }
        }
        let unpacked = self.dir.join("unpacked");
        run("/usr/bin/ditto", &["-x", "-k", path(&zip), path(&unpacked)]).await.context("could not unpack the update")?;
        let new_app = std::fs::read_dir(&unpacked)?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or_else(|| anyhow!("the download has no app in it"))?;
        let plist = std::fs::read_to_string(new_app.join("Contents/Info.plist")).unwrap_or_default();
        if !plist.contains(&format!("<string>{BUNDLE_ID}</string>")) {
            bail!("the download isn't Anagnost");
        }
        run("/usr/bin/codesign", &["--verify", "--deep", path(&new_app)]).await.context("the downloaded app's signature is invalid")?;

        let script = self.dir.join("swap.sh");
        std::fs::write(&script, SWAP_SCRIPT)?;
        let log = std::fs::File::create(self.dir.join("swap.log"))?;
        let mut cmd = std::process::Command::new("/bin/bash");
        let dest = bundle.parent().context("bad app location")?.join(new_app.file_name().context("bad app name")?);
        cmd.arg(&script)
            .arg(&new_app)
            .arg(&bundle)
            .arg(std::process::id().to_string())
            .arg(&dest)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        // Its own process group, so it outlives this app.
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        cmd.spawn().context("could not start the installer")?;
        Ok(())
    }
}

/// PDF Tutor was renamed Anagnost. Updates from 0.2.0 replace the app in place, so the bundle
/// keeps its old folder name; rename it once so Finder, Launchpad and Spotlight show the new one.
pub fn adopt_new_bundle_name() {
    let Some(bundle) = bundle_path() else { return };
    let Some(parent) = bundle.parent() else { return };
    let wanted = parent.join("Anagnost.app");
    if bundle.file_name().is_some_and(|n| n == "PDF Tutor.app") && !wanted.exists() {
        let _ = std::fs::rename(&bundle, &wanted);
    }
}

fn path(p: &Path) -> &str {
    p.to_str().unwrap_or_default()
}

async fn run(program: &str, args: &[&str]) -> Result<()> {
    let out = tokio::process::Command::new(program).args(args).output().await?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// "https://github.com/owner/repo" -> "owner/repo"
fn repo() -> Option<&'static str> {
    REPOSITORY.strip_prefix("https://github.com/").map(|r| r.trim_end_matches('/')).filter(|r| r.contains('/'))
}

fn parse_release(v: &Value) -> Release {
    let version = v["tag_name"].as_str().unwrap_or_default().trim_start_matches('v').to_string();
    let notes = v["body"]
        .as_str()
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().strip_prefix("* ").or_else(|| l.trim().strip_prefix("- ")))
        .map(|l| l.split(" by @").next().unwrap_or(l).trim().to_string())
        .filter(|l| !l.is_empty())
        .take(6)
        .collect();
    let assets: Vec<(&str, &str)> = v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a["name"].as_str()?, a["browser_download_url"].as_str()?)))
        .collect();
    let asset = assets.iter().find(|(n, _)| n.contains("macos") && n.ends_with(".zip")).map(|(name, url)| {
        let sha = assets.iter().find(|(n, _)| *n == format!("{name}.sha256")).map(|(_, u)| u.to_string());
        (url.to_string(), sha)
    });
    Release { version, notes, url: v["html_url"].as_str().unwrap_or_default().to_string(), downloadable: asset.is_some(), asset }
}

/// Plain x.y.z comparison; a missing or odd part counts as 0.
fn is_newer(candidate: &str, current: &str) -> bool {
    let parts = |s: &str| -> Vec<u64> { s.split(['.', '-']).take(3).map(|p| p.parse().unwrap_or(0)).collect() };
    parts(candidate) > parts(current)
}

fn unavailable() -> Option<String> {
    let Some(bundle) = bundle_path() else {
        return Some("Updating works in the installed app, not in development builds.".into());
    };
    let parent = bundle.parent()?;
    let probe = parent.join(".anagnost-write-test");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(probe);
            None
        }
        Err(_) => Some(format!("Anagnost can't replace itself in {} (no write access). Download the new version from GitHub instead.", parent.display())),
    }
}

/// …/Anagnost.app for the running executable, if it is inside an app bundle.
fn bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bundle = exe.parent()?.parent()?.parent()?;
    (bundle.extension()? == "app").then(|| bundle.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hits github.com: cargo test -- --ignored finds_latest_release_without_the_api
    #[tokio::test]
    #[ignore]
    async fn finds_latest_release_without_the_api() {
        let r = Updater::latest_from_web(&reqwest::Client::new(), "AlexShavkuta/anagnost").await.unwrap().unwrap();
        assert!(is_newer(&r.version, "0.2.0"), "{}", r.version);
        assert!(r.downloadable, "{}", r.url);
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.3"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
    }

    #[test]
    fn reads_generated_release_notes_and_assets() {
        let v = serde_json::json!({
            "tag_name": "v0.3.0",
            "html_url": "https://github.com/o/r/releases/tag/v0.3.0",
            "body": "## What's Changed\n* Faster page jumps by @someone in https://github.com/o/r/pull/3\n* Update popup by @someone in https://github.com/o/r/pull/4\n\n**Full Changelog**: https://github.com/o/r/compare/v0.2.0...v0.3.0",
            "assets": [
                {"name": "Anagnost-0.3.0-macos-universal.zip", "browser_download_url": "https://x/a.zip"},
                {"name": "Anagnost-0.3.0-macos-universal.zip.sha256", "browser_download_url": "https://x/a.zip.sha256"}
            ]
        });
        let r = parse_release(&v);
        assert_eq!(r.version, "0.3.0");
        assert_eq!(r.notes, ["Faster page jumps", "Update popup"]);
        assert_eq!(r.asset, Some(("https://x/a.zip".into(), Some("https://x/a.zip.sha256".into()))));
    }
}
