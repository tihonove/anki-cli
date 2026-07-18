//! Integration tests for local (offline) operations, driven through the
//! compiled binary the same way an agent would use it.

use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;

fn cli(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("anki-cli").unwrap();
    cmd.arg("--dir").arg(dir);
    cmd
}

#[test]
fn add_search_show_edit_rm_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    cli(dir)
        .args(["add", "-d", "Spanish", "-m", "Basic", "hola", "привет", "-t", "greeting a1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added note"));

    let out = cli(dir)
        .args(["--json", "search", "deck:Spanish"])
        .assert()
        .success();
    let notes: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).unwrap();
    let notes = notes.as_array().unwrap();
    assert_eq!(notes.len(), 1);
    let note = &notes[0];
    assert_eq!(note["fields"][0]["value"], "hola");
    assert_eq!(note["fields"][1]["value"], "привет");
    assert_eq!(note["tags"], serde_json::json!(["a1", "greeting"]));
    assert_eq!(note["cards"][0]["deck"], "Spanish");
    let nid = note["note_id"].as_i64().unwrap().to_string();

    cli(dir)
        .args(["show", &nid])
        .assert()
        .success()
        .stdout(predicate::str::contains("hola"));

    cli(dir)
        .args(["edit", &nid, "--field", "Back=привет!", "--add-tags", "checked", "--remove-tags", "a1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("привет!").and(predicate::str::contains("checked")));

    cli(dir)
        .args(["rm", &nid])
        .assert()
        .success();
    cli(dir)
        .args(["search", "deck:Spanish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No notes found"));
}

#[test]
fn add_with_named_fields_and_unknown_field_error() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    cli(dir)
        .args(["add", "--field", "Back=b", "--field", "Front=f"])
        .assert()
        .success();

    cli(dir)
        .args(["add", "--field", "Nope=x", "--field", "Front=f"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("has no field 'Nope'"));

    // malformed field syntax
    cli(dir)
        .args(["add", "--field", "no-equals-sign"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Name=Value"));
}

#[test]
fn decks_and_models_listing() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    cli(dir)
        .args(["add", "-d", "Deutsch::A1", "der Hund", "собака"])
        .assert()
        .success();

    cli(dir)
        .args(["decks"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deutsch::A1").and(predicate::str::contains("(1 cards)")));

    cli(dir)
        .args(["models"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Basic").and(predicate::str::contains("Cloze")));

    cli(dir)
        .args(["--json", "models", "Basic"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"Front\"").and(predicate::str::contains("\"Back\"")));
}

#[test]
fn status_offline_reports_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    cli(dir)
        .args(["add", "front", "back"])
        .assert()
        .success();

    let out = cli(dir)
        .args(["--json", "status", "--offline"])
        .assert()
        .success();
    let report: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(report["notes"], 1);
    assert_eq!(report["cards"], 1);
    assert_eq!(report["remote"], "offline");
}

#[test]
fn sync_without_login_fails_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    cli(dir)
        .args(["sync"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not logged in"));

    // and in JSON mode the error is JSON on stderr
    let out = cli(dir).args(["--json", "sync"]).assert().failure();
    let err: serde_json::Value =
        serde_json::from_slice(&out.get_output().stderr).unwrap();
    assert!(err["error"].as_str().unwrap().contains("not logged in"));
}

#[test]
fn sync_media_without_login_fails_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    cli(dir)
        .args(["sync-media"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not logged in"));

    let out = cli(dir).args(["--json", "sync-media"]).assert().failure();
    let err: serde_json::Value =
        serde_json::from_slice(&out.get_output().stderr).unwrap();
    assert!(err["error"].as_str().unwrap().contains("not logged in"));
}

#[test]
fn init_and_walk_up_resolution() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // init creates .anki with a gitignore
    Command::cargo_bin("anki-cli")
        .unwrap()
        .current_dir(root)
        .env_remove("ANKI_CLI_HOME")
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized"));
    assert!(root.join(".anki/.gitignore").exists());

    // re-init refuses
    Command::cargo_bin("anki-cli")
        .unwrap()
        .current_dir(root)
        .env_remove("ANKI_CLI_HOME")
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));

    // commands run from a nested subdirectory find the collection up the tree
    let nested = root.join("a/b");
    std::fs::create_dir_all(&nested).unwrap();
    Command::cargo_bin("anki-cli")
        .unwrap()
        .current_dir(&nested)
        .env_remove("ANKI_CLI_HOME")
        .args(["add", "front", "back"])
        .assert()
        .success();
    assert!(root.join(".anki/collection.anki2").exists());

    // config lands inside .anki with private permissions
    Command::cargo_bin("anki-cli")
        .unwrap()
        .current_dir(&nested)
        .env_remove("ANKI_CLI_HOME")
        .arg("logout")
        .assert()
        .success();
    let config = root.join(".anki/config.json");
    assert!(config.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&config).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn uninitialized_directory_fails_with_hint() {
    let tmp = tempfile::tempdir().unwrap();

    Command::cargo_bin("anki-cli")
        .unwrap()
        .current_dir(tmp.path())
        .env_remove("ANKI_CLI_HOME")
        .args(["status", "--offline"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("anki-cli init"));
}

#[test]
fn rm_nonexistent_note_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    cli(dir)
        .args(["rm", "12345"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no note with id 12345"));
}

#[test]
fn mcp_server_stdio_flow() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    let requests = concat!(
        r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#, "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, "\n",
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"anki_add_note","arguments":{"deck":"D","fields":{"Front":"f","Back":"b"},"tags":["t1"]}}}"#, "\n",
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"anki_search","arguments":{"query":"tag:t1"}}}"#, "\n",
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"anki_get_note","arguments":{"note_id":999}}}"#, "\n",
    );
    let out = cli(dir).arg("mcp").write_stdin(requests).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let responses: Vec<serde_json::Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // 5 requests with ids get responses; the notification does not.
    assert_eq!(responses.len(), 5);

    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "anki-cli");
    let tools = responses[1]["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "anki_sync"));

    assert_eq!(responses[2]["result"]["isError"], false);
    let added: serde_json::Value =
        serde_json::from_str(responses[2]["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap();
    assert_eq!(added["fields"][0]["value"], "f");

    let found: serde_json::Value =
        serde_json::from_str(responses[3]["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap();
    assert_eq!(found.as_array().unwrap().len(), 1);

    // bad note id surfaces as a tool error, not a crash
    assert_eq!(responses[4]["result"]["isError"], true);
}

#[test]
fn cloze_notetype_works() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    cli(dir)
        .args(["add", "-m", "Cloze", "Der {{c1::Hund}} bellt."])
        .assert()
        .success();

    cli(dir)
        .args(["search", "Hund"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hund"));
}
