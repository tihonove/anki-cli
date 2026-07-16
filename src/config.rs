use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Persistent CLI state: credentials and sync endpoint.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    pub username: Option<String>,
    pub hkey: Option<String>,
    /// Custom sync server endpoint; None means AnkiWeb.
    pub endpoint: Option<String>,
}

impl Config {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("config.json")
    }

    pub fn load(dir: &Path) -> Result<Config> {
        let path = Self::path(dir);
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;
        let path = Self::path(dir);
        fs::write(&path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

/// Resolve the data directory: --dir flag > ANKI_CLI_HOME env > ~/.local/share/anki-cli
pub fn resolve_dir(flag: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = flag {
        return Ok(dir);
    }
    if let Ok(env_dir) = std::env::var("ANKI_CLI_HOME") {
        if !env_dir.is_empty() {
            return Ok(PathBuf::from(env_dir));
        }
    }
    let base = dirs::data_dir().context("cannot determine data directory")?;
    Ok(base.join("anki-cli"))
}

pub fn collection_path(dir: &Path) -> PathBuf {
    dir.join("collection.anki2")
}
