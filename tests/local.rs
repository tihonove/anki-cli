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
