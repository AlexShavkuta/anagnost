//! API keys, kept in a file only this macOS user can read (mode 0600) in the app's data folder.
//!
//! Not the Keychain: this app is built locally and only ad-hoc signed, so every rebuild looks
//! like a new app to the Keychain, which then asks for the login password on every read.
//! The keys are read once at launch and kept in memory.

use anyhow::Result;
use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const ANTHROPIC: &str = "anthropic-api-key";
pub const CURSOR: &str = "cursor-api-key";

pub struct Secrets {
    path: PathBuf,
    values: Mutex<BTreeMap<String, String>>,
}

impl Secrets {
    pub fn open(dir: &Path) -> Result<Self> {
        let path = dir.join("keys.json");
        let values = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, values: Mutex::new(values) })
    }

    pub fn get(&self, name: &str) -> Option<String> {
        self.values.lock().unwrap().get(name).filter(|k| !k.is_empty()).cloned()
    }

    /// Save a key; an empty one removes it.
    pub fn set(&self, name: &str, key: &str) -> Result<()> {
        let mut values = self.values.lock().unwrap();
        let mut next = values.clone();
        if key.is_empty() {
            next.remove(name);
        } else {
            next.insert(name.to_string(), key.to_string());
        }
        write_private(&self.path, &next)?;
        *values = next;
        Ok(())
    }
}

/// Write via a temp file created with mode 0600, then rename over the old file.
fn write_private(path: &Path, values: &BTreeMap<String, String>) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    f.write_all(serde_json::to_string_pretty(values)?.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn saves_privately_and_reloads() {
        let dir = std::env::temp_dir().join(format!("anagnost-secrets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = Secrets::open(&dir).unwrap();
        assert_eq!(s.get(ANTHROPIC), None);
        s.set(ANTHROPIC, "sk-test").unwrap();
        let mode = std::fs::metadata(dir.join("keys.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(Secrets::open(&dir).unwrap().get(ANTHROPIC).as_deref(), Some("sk-test"));
        s.set(ANTHROPIC, "").unwrap();
        assert_eq!(Secrets::open(&dir).unwrap().get(ANTHROPIC), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
