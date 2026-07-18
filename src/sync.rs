use std::path::Path;

use anki::sync::collection::normal::SyncActionRequired;
use anki::sync::collection::status::online_sync_status_check;
use anki::sync::http_client::HttpSyncClient;
use anki::sync::login::sync_login;
use anki::sync::media::progress::MediaSyncProgress;
use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::col::{auth_from_config, http_client, open_collection};
use crate::config::Config;

pub async fn login(
    dir: &Path,
    config: &mut Config,
    username: &str,
    password: &str,
    endpoint: Option<String>,
) -> Result<()> {
    let auth = sync_login(username, password, endpoint.clone(), http_client())
        .await
        .map_err(|e| anyhow::anyhow!("login failed: {}", e.message(&anki::prelude::I18n::template_only())))?;
    config.username = Some(username.to_string());
    config.hkey = Some(auth.hkey);
    if endpoint.is_some() {
        config.endpoint = endpoint;
    }
    config.save(dir)?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct SyncReport {
    /// "up_to_date" | "synced" | "conflict"
    pub result: String,
    pub server_message: Option<String>,
    /// Set when result == "conflict"
    pub hint: Option<String>,
}

/// Two-way sync. On divergence returns a "conflict" report and exit code 2 is
/// used by the caller.
pub async fn normal_sync(dir: &Path, config: &mut Config) -> Result<SyncReport> {
    let auth = auth_from_config(config)?;
    let mut col = open_collection(dir)?;

    // Detect the no-op case so we can report it distinctly: if neither side
    // changed anything, the collection's modification stamp stays put.
    let modified_before = col.sync_meta()?.modified;

    let out = col
        .normal_sync(auth, http_client())
        .await
        .map_err(|e| anyhow::anyhow!("sync failed: {}", e.message(&anki::prelude::I18n::template_only())))?;

    let changed = col.sync_meta()?.modified != modified_before;

    if let Some(new_endpoint) = &out.new_endpoint {
        config.endpoint = Some(new_endpoint.clone());
        config.save(dir)?;
    }

    let server_message = (!out.server_message.is_empty()).then(|| out.server_message.clone());
    match out.required {
        SyncActionRequired::NoChanges => Ok(SyncReport {
            result: if changed { "synced" } else { "up_to_date" }.into(),
            server_message,
            hint: None,
        }),
        SyncActionRequired::FullSyncRequired {
            upload_ok,
            download_ok,
        } => {
            let mut options = Vec::new();
            if download_ok {
                options.push("`anki-cli pull` (take server version, discard local changes)");
            }
            if upload_ok {
                options.push("`anki-cli push` (take local version, overwrite server)");
            }
            Ok(SyncReport {
                result: "conflict".into(),
                server_message,
                hint: Some(format!(
                    "collections have diverged and cannot be merged; resolve with {}",
                    options.join(" or ")
                )),
            })
        }
        SyncActionRequired::NormalSyncRequired => unreachable!("sync just completed"),
    }
}

/// Resolve the sync shard for full transfers: AnkiWeb's base host redirects
/// meta requests to e.g. sync11.ankiweb.net, but full up/downloads don't
/// follow that redirect, so it must be baked into the auth first.
async fn auth_with_resolved_endpoint(
    dir: &Path,
    config: &mut Config,
    col: &anki::collection::Collection,
) -> Result<anki::sync::login::SyncAuth> {
    let mut auth = auth_from_config(config)?;
    let mut client = HttpSyncClient::new(auth.clone(), http_client());
    let meta = col.sync_meta()?;
    let state = online_sync_status_check(meta, &mut client)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "checking server status: {}",
                e.message(&anki::prelude::I18n::template_only())
            )
        })?;
    if let Some(ep) = &state.new_endpoint {
        config.endpoint = Some(ep.clone());
        config.save(dir)?;
        auth = auth_from_config(config)?;
    }
    Ok(auth)
}

/// Full download: replace local collection with the server's copy.
pub async fn pull(dir: &Path, config: &mut Config, force: bool) -> Result<()> {
    let mut col = open_collection(dir)?;

    if !force {
        let mut pending = col.sync_status_offline()?
            != anki_proto::sync::sync_status_response::Required::NoChanges;
        // A never-synced collection reports NoChanges regardless of content,
        // so guard its notes separately.
        if !pending && col.sync_meta()?.usn.0 == 0 {
            pending = !col.search_notes_unordered("")?.is_empty();
        }
        if pending {
            bail!(
                "local collection has unsynced changes that `pull` would discard; \
                 run `anki-cli sync` first, or `anki-cli pull --force` to discard them"
            );
        }
    }

    let auth = auth_with_resolved_endpoint(dir, config, &col).await?;
    let tr = anki::prelude::I18n::template_only();
    col.full_download(auth, http_client())
        .await
        .map_err(|e| anyhow::anyhow!("download failed: {}", e.message(&tr)))?;
    Ok(())
}

/// Full upload: replace the server collection with the local copy.
pub async fn push(dir: &Path, config: &mut Config) -> Result<()> {
    let col = open_collection(dir)?;
    let auth = auth_with_resolved_endpoint(dir, config, &col).await?;
    let tr = anki::prelude::I18n::template_only();
    col.full_upload(auth, http_client())
        .await
        .map_err(|e| anyhow::anyhow!("upload failed: {}", e.message(&tr)))?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct MediaSyncReport {
    /// "synced"
    pub result: String,
    /// Number of media files in the local media folder after the sync.
    pub media_files: usize,
}

/// Sync media files (images, audio, …) with the server: uploads locally-added
/// files and downloads server-side additions/deletions. Unlike collection sync
/// this merges file-by-file and never conflicts, so it needs no push/pull
/// resolution. Media lives alongside the collection in `<dir>/collection.media`.
pub async fn sync_media(dir: &Path, config: &mut Config) -> Result<MediaSyncReport> {
    let col = open_collection(dir)?;
    // Media transfers hit the same sync shard as full up/downloads, so resolve
    // the redirected endpoint first (AnkiWeb's base host won't serve them).
    let auth = auth_with_resolved_endpoint(dir, config, &col).await?;
    let tr = anki::prelude::I18n::template_only();

    let mgr = col
        .media()
        .map_err(|e| anyhow::anyhow!("opening media folder: {}", e.message(&tr)))?;
    let progress = col.new_progress_handler::<MediaSyncProgress>();
    // `server_usn = None` lets the syncer discover it via begin().
    mgr.sync_media(progress, auth, http_client(), None)
        .await
        .map_err(|e| anyhow::anyhow!("media sync failed: {}", e.message(&tr)))?;

    Ok(MediaSyncReport {
        result: "synced".into(),
        media_files: count_media_files(dir),
    })
}

/// Count regular files in the local media folder (`<dir>/collection.media`).
fn count_media_files(dir: &Path) -> usize {
    let folder = crate::config::collection_path(dir).with_extension("media");
    std::fs::read_dir(&folder)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .count()
        })
        .unwrap_or(0)
}

#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub collection_exists: bool,
    pub notes: usize,
    pub cards: usize,
    pub local_changes: bool,
    /// "up_to_date" | "sync_needed" | "conflict" | "offline" (not checked)
    pub remote: String,
    pub server_message: Option<String>,
}

pub async fn status(dir: &Path, config: &Config, offline: bool) -> Result<StatusReport> {
    let exists = crate::config::collection_path(dir).exists();
    let mut col = open_collection(dir)?;
    let notes = col.search_notes_unordered("")?.len();
    let cards = col
        .search_cards("", anki::search::SortMode::NoOrder)?
        .len();
    let local_changes = col.sync_status_offline()?
        != anki_proto::sync::sync_status_response::Required::NoChanges;

    let (remote, server_message) = if offline {
        ("offline".to_string(), None)
    } else {
        let auth = auth_from_config(config)?;
        let meta = col.sync_meta().context("reading local sync metadata")?;
        let mut client = HttpSyncClient::new(auth, http_client());
        let state = online_sync_status_check(meta, &mut client)
            .await
            .map_err(|e| anyhow::anyhow!("checking server status: {}", e.message(&anki::prelude::I18n::template_only())))?;
        let remote = match state.required {
            SyncActionRequired::NoChanges => "up_to_date",
            SyncActionRequired::NormalSyncRequired => "sync_needed",
            SyncActionRequired::FullSyncRequired { .. } => "conflict",
        };
        let msg = (!state.server_message.is_empty()).then(|| state.server_message.clone());
        (remote.to_string(), msg)
    };

    Ok(StatusReport {
        collection_exists: exists,
        notes,
        cards,
        local_changes,
        remote,
        server_message,
    })
}
