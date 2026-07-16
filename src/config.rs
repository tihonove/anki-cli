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
        // The file holds the AnkiWeb session key — keep it private.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

pub const DATA_DIR_NAME: &str = ".anki";

/// Resolve the data directory: --dir flag > ANKI_CLI_HOME env > nearest
/// `.anki/` directory walking up from the current directory (git-style).
pub fn resolve_dir(flag: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = flag {
        return Ok(dir);
    }
    if let Ok(env_dir) = std::env::var("ANKI_CLI_HOME") {
        if !env_dir.is_empty() {
            return Ok(PathBuf::from(env_dir));
        }
    }
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    for ancestor in cwd.ancestors() {
        let candidate = ancestor.join(DATA_DIR_NAME);
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    anyhow::bail!(
        "no {DATA_DIR_NAME}/ directory found here or in any parent directory; \
         run `anki-cli init` to start a collection here (or pass --dir)"
    )
}

/// Create `.anki/` in `base`, ready for login+pull. Fails if already present.
pub fn init_dir(base: &Path) -> Result<PathBuf> {
    let dir = base.join(DATA_DIR_NAME);
    if dir.exists() {
        anyhow::bail!("{} already exists", dir.display());
    }
    init_dir_at(&dir)?;
    Ok(dir)
}

/// Turn `dir` itself into a data directory (used with an explicit --dir).
pub fn init_dir_at(dir: &Path) -> Result<PathBuf> {
    if collection_path(dir).exists() || Config::path(dir).exists() {
        anyhow::bail!("{} already holds a collection", dir.display());
    }
    fs::create_dir_all(dir)?;
    // The collection and session key have no place in version control.
    fs::write(dir.join(".gitignore"), "*\n")?;
    Ok(dir.to_path_buf())
}

pub fn collection_path(dir: &Path) -> PathBuf {
    dir.join("collection.anki2")
}
