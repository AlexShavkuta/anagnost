//! Self-update for copies built from a git clone with scripts/install-macos.sh:
//! the build bakes in the clone's folder and commit. "Check" fetches the clone's remote;
//! "Update" runs git pull and the install script in a detached process, which replaces
//! the app and opens it again.

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const SOURCE: &str = env!("PDF_TUTOR_SOURCE");
const COMMIT: &str = env!("PDF_TUTOR_COMMIT");
const COMMIT_DATE: &str = env!("PDF_TUTOR_COMMIT_DATE");
const BRANCH: &str = "main";

const SCRIPT: &str = r#"#!/bin/bash
# Written by PDF Tutor's Update button. $1: source folder, $2: the app to reopen.
set -euo pipefail
# Apps opened from Finder don't get the shell's PATH; borrow the login shell's.
PATH="$(/bin/zsh -lc 'printf %s "$PATH"' 2>/dev/null || true):$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
export PATH
cd "$1"
echo "::step Downloading the update"
git pull --ff-only origin main
echo "::step Building (this takes a few minutes)"
bash scripts/install-macos.sh
echo "::step Restarting"
open "$2"
"#;

#[derive(Serialize)]
pub struct Info {
    pub commit: &'static str,
    pub commit_date: &'static str,
    pub source: &'static str,
    /// Why updating from inside the app won't work, if it won't.
    pub unavailable: Option<String>,
}

#[derive(Serialize)]
pub struct Commit {
    pub sha: String,
    pub message: String,
}

#[derive(Serialize)]
pub struct Check {
    /// New commits on GitHub that this build doesn't have, newest first.
    pub commits: Vec<Commit>,
    pub behind: u64,
}

#[derive(Serialize, Clone)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Idle,
    Running { step: String },
    Error { message: String },
}

pub struct Updater {
    app: AppHandle,
    log_path: PathBuf,
    script_path: PathBuf,
    status: Mutex<Status>,
}

impl Updater {
    pub fn new(app: AppHandle, data_dir: &Path) -> Self {
        Self {
            app,
            log_path: data_dir.join("update.log"),
            script_path: data_dir.join("update.sh"),
            status: Mutex::new(Status::Idle),
        }
    }

    pub fn info(&self) -> Info {
        Info { commit: COMMIT, commit_date: COMMIT_DATE, source: SOURCE, unavailable: unavailable() }
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    fn set_status(&self, s: Status) {
        *self.status.lock().unwrap() = s.clone();
        let _ = self.app.emit("update-status", s);
    }

    /// Fetch the clone's remote and list the commits on main that this build doesn't have.
    pub async fn check(&self) -> Result<Check> {
        if let Some(why) = unavailable() {
            bail!(why);
        }
        if COMMIT.is_empty() {
            bail!("This copy doesn't know which commit it was built from.");
        }
        git(&["fetch", "--quiet", "origin", BRANCH]).await.context("Could not fetch updates")?;
        let range = format!("{COMMIT}..origin/{BRANCH}");
        let behind = git(&["rev-list", "--count", &range]).await?.trim().parse().unwrap_or(0);
        let log = git(&["log", "--no-merges", "--format=%H%x09%s", &range]).await?;
        let commits = log
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .map(|(sha, message)| Commit { sha: sha.into(), message: message.into() })
            .collect();
        Ok(Check { behind, commits })
    }

    /// Start the update in a process that outlives this app (the install script quits it).
    pub fn start(&self) -> Result<()> {
        if matches!(self.status(), Status::Running { .. }) {
            return Ok(());
        }
        if let Some(why) = unavailable() {
            bail!(why);
        }
        let bundle = bundle_path().ok_or_else(|| anyhow!("PDF Tutor isn't running from an installed app."))?;
        std::fs::write(&self.script_path, SCRIPT)?;
        let log = std::fs::File::create(&self.log_path)?;
        let mut cmd = std::process::Command::new("/bin/bash");
        cmd.env("PDF_TUTOR_PID", std::process::id().to_string())
            .arg(&self.script_path)
            .arg(SOURCE)
            .arg(&bundle)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        // Its own process group, so quitting the app doesn't take the update down with it.
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd.spawn().context("could not start the update")?;
        self.set_status(Status::Running { step: "Starting the update".into() });

        let log_path = self.log_path.clone();
        let app = self.app.clone();
        std::thread::spawn(move || {
            let updater = app.state::<std::sync::Arc<crate::state::AppState>>();
            let updater = &updater.updater;
            loop {
                let log = std::fs::read_to_string(&log_path).unwrap_or_default();
                if let Some(step) = log.lines().rev().find_map(|l| l.strip_prefix("::step ")) {
                    if step == "Installing" {
                        // The new build is ready; get out of the way so it can replace this one.
                        app.exit(0);
                        return;
                    }
                    if !matches!(updater.status(), Status::Running { step: ref s } if s == step) {
                        updater.set_status(Status::Running { step: step.to_string() });
                    }
                }
                match child.try_wait() {
                    Ok(Some(code)) => {
                        if code.success() {
                            updater.set_status(Status::Error {
                                message: "The update is installed. Quit and reopen PDF Tutor to use it.".into(),
                            });
                        } else {
                            let tail: Vec<&str> = log.lines().filter(|l| !l.trim().is_empty()).collect();
                            let tail = tail[tail.len().saturating_sub(8)..].join("\n");
                            updater.set_status(Status::Error { message: format!("The update failed.\n{tail}") });
                        }
                        break;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(800)),
                    Err(e) => {
                        updater.set_status(Status::Error { message: format!("Lost track of the update: {e}") });
                        break;
                    }
                }
            }
        });
        Ok(())
    }
}


/// Run git in the source folder; never prompts (a prompt would hang with no terminal).
async fn git(args: &[&str]) -> Result<String> {
    let out = tokio::process::Command::new("/usr/bin/git")
        .arg("-C")
        .arg(SOURCE)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(60), out).await.context("git timed out")??;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn unavailable() -> Option<String> {
    let src = Path::new(SOURCE);
    if !src.join(".git").exists() || !src.join("scripts/install-macos.sh").is_file() {
        return Some(format!(
            "The source folder this copy was built from ({SOURCE}) isn't there anymore. \
             Clone the repo again and run npm run install-app."
        ));
    }
    if bundle_path().is_none() {
        return Some("Updating works in the installed app, not in development builds.".into());
    }
    None
}

/// …/PDF Tutor.app for the running executable, if it is inside an app bundle.
fn bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bundle = exe.parent()?.parent()?.parent()?;
    (bundle.extension()? == "app").then(|| bundle.to_path_buf())
}
