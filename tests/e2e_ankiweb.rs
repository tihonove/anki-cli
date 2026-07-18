//! End-to-end tests that drive the built binary against a **real AnkiWeb
//! account**, across two separate data directories: device A authors a note +
//! media file and pushes it, device B pulls and must see both.
//!
//! Gated on `ANKI_TEST_USERNAME` / `ANKI_TEST_PASSWORD`: with no creds the test
//! self-skips, so `cargo test` stays hermetic locally and in the offline build
//! job. CI sets the creds (see `.github/workflows/ci.yml`) and runs this file
//! with `--test-threads=1`, so the shared account is only ever touched by one
//! test at a time.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use assert_cmd::Command;

/// Fixed media filename + content, on purpose: identical bytes across runs mean
/// the same sha1, so the server never accumulates media (re-upload is a no-op)
/// and no destructive cleanup is needed — each run's `push` replaces the whole
/// collection anyway. The `.png` extension is cosmetic; media sync transfers
/// raw bytes and doesn't parse the file.
const MEDIA_NAME: &str = "anki-cli-e2e.png";
const MEDIA_BYTES: &[u8] = b"anki-cli e2e media payload v1\n";

/// Test credentials from the environment, or `None` to skip.
fn creds() -> Option<(String, String)> {
    let user = std::env::var("ANKI_TEST_USERNAME").ok().filter(|s| !s.is_empty())?;
    let pass = std::env::var("ANKI_TEST_PASSWORD").ok().filter(|s| !s.is_empty())?;
    Some((user, pass))
}

/// A binary invocation bound to `dir`, with creds injected into the child env
/// as `ANKI_USERNAME` / `ANKI_PASSWORD` (which `login` reads). Passing creds via
/// env rather than argv keeps the password off the command line, so it can't
/// leak through assert_cmd's command echo in CI logs on failure.
fn cli(dir: &Path, user: &str, pass: &str) -> Command {
    let mut cmd = Command::cargo_bin("anki-cli").unwrap();
    cmd.arg("--dir").arg(dir);
    cmd.env("ANKI_USERNAME", user).env("ANKI_PASSWORD", pass);
    cmd
}

#[test]
fn ankiweb_round_trip() {
    let Some((user, pass)) = creds() else {
        eprintln!(
            "skipping ankiweb_round_trip: set ANKI_TEST_USERNAME / ANKI_TEST_PASSWORD to run it"
        );
        return;
    };

    // A token unique to this run, so device B provably reads *this* run's data
    // rather than a leftover collection on the shared account.
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let run: String = std::env::var("GITHUB_RUN_ID")
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    let token = format!("e2etoken{run}p{}n{nanos}", std::process::id());

    let tmp_a = tempfile::tempdir().unwrap();
    let tmp_b = tempfile::tempdir().unwrap();
    let dir_a = tmp_a.path();
    let dir_b = tmp_b.path();

    // --- Device A: init, login, author a note + media, push, upload media ----
    cli(dir_a, &user, &pass).arg("init").assert().success();
    cli(dir_a, &user, &pass).arg("login").assert().success();

    let front = format!("{token} <img src=\"{MEDIA_NAME}\">");
    cli(dir_a, &user, &pass)
        .args(["add", "-d", "Default", "-m", "Basic", &front, "e2e back", "-t", "anki-cli-e2e"])
        .assert()
        .success();

    // Drop the media file into A's media folder before uploading it.
    let media_a = dir_a.join("collection.media");
    std::fs::create_dir_all(&media_a).unwrap();
    std::fs::write(media_a.join(MEDIA_NAME), MEDIA_BYTES).unwrap();

    // Full upload: the server collection becomes A's copy — deterministic, and
    // works even if the account was never synced before.
    cli(dir_a, &user, &pass).arg("push").assert().success();
    cli(dir_a, &user, &pass).arg("sync-media").assert().success();

    // The online status path should report up-to-date right after a push.
    let out = cli(dir_a, &user, &pass).args(["--json", "status"]).assert().success();
    let status: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(status["remote"], "up_to_date", "A should be up to date right after push");

    // --- Device B: init, login, pull, download media, verify -----------------
    cli(dir_b, &user, &pass).arg("init").assert().success();
    cli(dir_b, &user, &pass).arg("login").assert().success();
    cli(dir_b, &user, &pass).arg("pull").assert().success();
    cli(dir_b, &user, &pass).arg("sync-media").assert().success();

    // The note authored on A must be found on B by its unique token.
    let out = cli(dir_b, &user, &pass).args(["--json", "search", &token]).assert().success();
    let notes: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let notes = notes.as_array().expect("search returns a JSON array");
    assert_eq!(notes.len(), 1, "device B should find exactly the note A pushed (token {token})");
    assert!(
        notes[0]["fields"][0]["value"].as_str().unwrap().contains(&token),
        "the found note's Front should carry the run token"
    );

    // The media file authored on A must have downloaded to B, byte-for-byte.
    let got = std::fs::read(dir_b.join("collection.media").join(MEDIA_NAME))
        .unwrap_or_else(|e| panic!("media file {MEDIA_NAME} missing on device B: {e}"));
    assert_eq!(got, MEDIA_BYTES, "media bytes on B must match what A uploaded");
}
