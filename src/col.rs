use std::path::Path;

use anki::collection::{Collection, CollectionBuilder};
use anki::sync::login::SyncAuth;
use anyhow::{bail, Context, Result};
use reqwest::Url;

use crate::config::{collection_path, Config};

pub fn open_collection(dir: &Path) -> Result<Collection> {
    std::fs::create_dir_all(dir)?;
    let path = collection_path(dir);
    CollectionBuilder::new(&path)
        .with_desktop_media_paths()
        .build()
        .with_context(|| format!("opening collection at {}", path.display()))
}

pub fn auth_from_config(config: &Config) -> Result<SyncAuth> {
    let Some(hkey) = config.hkey.clone() else {
        bail!("not logged in — run `anki-cli login -u <email> -p <password>` first");
    };
    let endpoint = match &config.endpoint {
        Some(ep) => Some(Url::parse(ep).with_context(|| format!("invalid endpoint {ep}"))?),
        None => None,
    };
    Ok(SyncAuth {
        hkey,
        endpoint,
        io_timeout_secs: None,
    })
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .use_native_tls()
        .build()
        .expect("building http client")
}
