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

/// Run a command and parse its `--json` output.
fn json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut cmd = cli(dir);
    cmd.arg("--json");
    let out = cmd.args(args).assert().success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

/// A deck whose notes carry a numeric `Rank` field, like the frequency-list
/// decks this tool is aimed at. Ranks are deliberately not in insertion order
/// and span one and two digits, so a lexicographic sort would get them wrong.
fn frequency_deck(dir: &Path, deck: &str, ranks: &[i32]) {
    cli(dir)
        .args(["models", "add-field", "Basic", "Rank", "--pos", "0"])
        .assert()
        .success();
    for rank in ranks {
        cli(dir)
            .args(["add", "-d", deck, "-m", "Basic"])
            .args(["--field", &format!("Rank={rank}")])
            .args(["--field", &format!("Front=w{rank}")])
            .args(["--field", &format!("Back=t{rank}")])
            .assert()
            .success();
    }
}

/// Rank values of the cards a query returns, in the order returned.
fn ranks_of(dir: &Path, args: &[&str]) -> Vec<i64> {
    let cards = json(dir, args);
    cards
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let note = json(dir, &["show", &c["note_id"].as_i64().unwrap().to_string()]);
            note["fields"][0]["value"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect()
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
        .stdout(
            predicate::str::contains("Deutsch::A1")
                .and(predicate::str::contains("1 cards: new 1")),
        );

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

/// `pull`'s guard against discarding local work must judge the collection's
/// *content*, not just rslib's offline sync status: a never-synced collection
/// (last sync = 0) reports FullSync from `sync_status_offline` no matter how
/// empty it is, so an untouched one used to be refused a pull. Offline proxy for
/// "the guard let us through": the run gets as far as the login check.
#[test]
fn pull_guard_lets_an_empty_collection_through() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    cli(dir).arg("init").assert().success();
    cli(dir)
        .arg("pull")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not logged in"));

    // …but a collection carrying a note of its own is held back.
    cli(dir)
        .args(["add", "-d", "Default", "-m", "Basic", "front", "back"])
        .assert()
        .success();
    cli(dir)
        .arg("pull")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsynced changes"));

    // --force skips the guard, so it too reaches the login check.
    cli(dir)
        .args(["pull", "--force"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not logged in"));
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
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"anki_suspend","arguments":{"query":"deck:D"}}}"#, "\n",
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"anki_cards","arguments":{"query":"deck:D"}}}"#, "\n",
    );
    let out = cli(dir).arg("mcp").write_stdin(requests).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let responses: Vec<serde_json::Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // 7 requests with ids get responses; the notification does not.
    assert_eq!(responses.len(), 7);

    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "anki-cli");

    // The agent-facing briefing: 28 tool descriptions don't add up to a mental
    // model, so the traps that aren't visible from any single tool have to be
    // stated up front.
    let instructions = responses[0]["result"]["instructions"].as_str().unwrap();
    for topic in ["anki_sync", "anki_cards", "sort_field", "dry_run", "anki_push"] {
        assert!(instructions.contains(topic), "instructions omit {topic}");
    }

    let tools = responses[1]["result"]["tools"].as_array().unwrap();
    for expected in [
        "anki_sync",
        "anki_cards",
        "anki_suspend",
        "anki_unsuspend",
        "anki_forget",
        "anki_reposition",
        "anki_move_cards",
        "anki_bulk_edit_notes",
        "anki_delete_deck",
        "anki_rename_deck",
        "anki_add_field",
        "anki_remove_field",
        "anki_move_field",
        "anki_edit_template",
    ] {
        assert!(tools.iter().any(|t| t["name"] == expected), "{expected} missing");
    }

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

    // a card-level op driven by a search query, and the state it produced
    let suspended: serde_json::Value =
        serde_json::from_str(responses[5]["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap();
    assert_eq!(suspended["changed"], 1);
    let cards: serde_json::Value =
        serde_json::from_str(responses[6]["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap();
    assert_eq!(cards[0]["queue"], "suspended");
}

#[test]
fn cards_command_reports_scheduling_state() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    cli(dir).args(["add", "-d", "Sched", "front", "back"]).assert().success();

    let cards = json(dir, &["cards", "deck:Sched"]);
    let card = &cards.as_array().unwrap()[0];
    assert_eq!(card["deck"], "Sched");
    assert_eq!(card["queue"], "new");
    assert_eq!(card["ctype"], "new");
    assert_eq!(card["ivl"], 0);
    assert_eq!(card["reps"], 0);

    cli(dir).args(["suspend", "deck:Sched"]).assert().success();
    let cards = json(dir, &["cards", "deck:Sched"]);
    assert_eq!(cards[0]["queue"], "suspended");

    cli(dir).args(["unsuspend", "deck:Sched"]).assert().success();
    let cards = json(dir, &["cards", "deck:Sched"]);
    assert_eq!(cards[0]["queue"], "new");
}

/// The core of the intended workflow: release the next N words by Rank.
/// Numeric, not lexicographic — with ranks 2 and 10 in play, sorting as text
/// would pick the wrong ones.
#[test]
fn suspend_by_query_honours_limit_and_numeric_field_sort() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    frequency_deck(dir, "Freq", &[10, 2, 33, 4]);

    let report = json(
        dir,
        &["suspend", "deck:Freq", "--sort-field", "Rank", "--limit", "2"],
    );
    assert_eq!(report["matched"], 2);
    assert_eq!(report["changed"], 2);

    assert_eq!(ranks_of(dir, &["cards", "deck:Freq is:suspended"]), vec![2, 4]);

    // …and releasing the next one by Rank takes the smallest still suspended.
    cli(dir)
        .args(["unsuspend", "deck:Freq is:suspended", "--sort-field", "Rank", "--limit", "1"])
        .assert()
        .success();
    assert_eq!(ranks_of(dir, &["cards", "deck:Freq is:suspended"]), vec![4]);
}

#[test]
fn dry_run_reports_without_changing_anything() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    frequency_deck(dir, "Freq", &[1, 2]);

    let report = json(dir, &["suspend", "deck:Freq", "--dry-run"]);
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["matched"], 2);
    assert_eq!(report["changed"], 0);
    assert_eq!(report["cards"].as_array().unwrap().len(), 2);

    let cards = json(dir, &["cards", "deck:Freq"]);
    assert!(cards.as_array().unwrap().iter().all(|c| c["queue"] == "new"));

    // The same for a schema change, which additionally reports what it costs.
    let report = json(dir, &["models", "rm-field", "Basic", "Rank", "--dry-run"]);
    assert_eq!(report["changed"], 0);
    assert_eq!(report["details"]["notes_losing_data"], 2);
    assert_eq!(report["full_sync_required"], true);
    let model = json(dir, &["models", "Basic"]);
    assert_eq!(model["fields"][0], "Rank");
}

#[test]
fn forget_resets_progress_and_selection_guards_hold() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    cli(dir).args(["add", "-d", "F", "a", "b"]).assert().success();

    let report = json(dir, &["forget", "deck:F", "--reset-counts"]);
    assert_eq!(report["changed"], 1);
    let cards = json(dir, &["cards", "deck:F"]);
    assert_eq!(cards[0]["ctype"], "new");
    assert_eq!(cards[0]["reps"], 0);

    // An empty query would sweep the whole collection, so it is refused.
    cli(dir)
        .args(["suspend", ""])
        .assert()
        .failure()
        .stderr(predicate::str::contains("empty search query"));
    cli(dir)
        .args(["suspend", "--ids", "999"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no card with id 999"));
}

#[test]
fn reposition_orders_the_new_queue_by_a_field() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    frequency_deck(dir, "Freq", &[10, 2, 33, 4]);

    cli(dir)
        .args(["reposition", "deck:Freq", "--sort-field", "Rank", "--start", "1"])
        .assert()
        .success();

    assert_eq!(
        ranks_of(dir, &["cards", "deck:Freq", "--sort", "position"]),
        vec![2, 4, 10, 33]
    );
}

#[test]
fn deck_removal_distinguishes_keeping_and_deleting_notes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    cli(dir).args(["add", "-d", "Keep", "a", "b"]).assert().success();
    cli(dir).args(["add", "-d", "Gone", "c", "d"]).assert().success();

    // Neither flag: refuse rather than guess, the two outcomes differ too much.
    cli(dir)
        .args(["decks", "rm", "Keep"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--with-notes"));

    let report = json(dir, &["decks", "rm", "Keep", "--keep-notes", "--to", "Archive"]);
    assert_eq!(report["details"]["cards_moved"], 1);
    assert_eq!(json(dir, &["cards", "deck:Archive"]).as_array().unwrap().len(), 1);

    let report = json(dir, &["decks", "rm", "Gone", "--with-notes"]);
    assert_eq!(report["details"]["notes_deleted"], 1);
    cli(dir)
        .args(["search", "deck:Gone"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No notes found"));
}

#[test]
fn deck_rename_carries_child_decks() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    cli(dir).args(["add", "-d", "Old::Sub", "a", "b"]).assert().success();

    cli(dir).args(["decks", "mv", "Old", "New"]).assert().success();
    let decks = json(dir, &["decks"]);
    let names: Vec<&str> = decks
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"New"), "{names:?}");
    assert!(names.contains(&"New::Sub"), "{names:?}");
}

#[test]
fn moving_cards_between_decks() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    frequency_deck(dir, "Freq", &[1, 2, 3]);

    let report = json(
        dir,
        &["mv", "deck:Freq", "--deck", "Later", "--sort-field", "Rank", "--reverse", "--limit", "1"],
    );
    assert_eq!(report["changed"], 1);
    assert_eq!(ranks_of(dir, &["cards", "deck:Later"]), vec![3]);

    cli(dir)
        .args(["mv", "deck:Freq", "--deck", "Nope", "--no-create"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no deck named 'Nope'"));
}

/// Adding a field must not shift the values of the existing ones — that is the
/// whole point of letting rslib migrate by field ordinal.
#[test]
fn notetype_field_editing_preserves_existing_values() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    cli(dir)
        .args(["add", "-d", "D", "-m", "Basic", "hola", "hello", "-t", "es"])
        .assert()
        .success();

    let report = json(dir, &["models", "add-field", "Basic", "Russian"]);
    assert_eq!(report["full_sync_required"], true);
    assert_eq!(report["details"]["fields"][2], "Russian");

    let notes = json(dir, &["search", "deck:D"]);
    let fields = &notes[0]["fields"];
    assert_eq!(fields[0]["value"], "hola");
    assert_eq!(fields[1]["value"], "hello");
    assert_eq!(fields[2]["value"], "");

    // Fill the new field, then move it and confirm the value travels with it.
    let nid = notes[0]["note_id"].as_i64().unwrap().to_string();
    cli(dir)
        .args(["edit", &nid, "--field", "Russian=привет"])
        .assert()
        .success();
    cli(dir)
        .args(["models", "mv-field", "Basic", "Russian", "--pos", "0"])
        .assert()
        .success();
    let notes = json(dir, &["search", "deck:D"]);
    assert_eq!(notes[0]["fields"][0]["name"], "Russian");
    assert_eq!(notes[0]["fields"][0]["value"], "привет");

    let report = json(dir, &["models", "rm-field", "Basic", "Russian"]);
    assert_eq!(report["details"]["fields"], serde_json::json!(["Front", "Back"]));
}

#[test]
fn card_template_editing() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let front = tmp.path().join("front.html");
    std::fs::write(&front, "{{Front}}<br>edited").unwrap();

    cli(dir).args(["add", "-d", "D", "a", "b"]).assert().success();
    let report = json(
        dir,
        &[
            "models",
            "edit-template",
            "Basic",
            "--card",
            "1",
            "--front",
            front.to_str().unwrap(),
        ],
    );
    assert_eq!(report["full_sync_required"], true);

    cli(dir)
        .args(["models", "edit-template", "Basic", "--card", "9", "--front", front.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("card template(s)"));
}

#[test]
fn bulk_edit_by_query() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    frequency_deck(dir, "Freq", &[1, 2, 3]);

    let report = json(
        dir,
        &["edit", "--query", "deck:Freq", "--add-tags", "nl::src::freq"],
    );
    assert_eq!(report["matched"], 3);
    assert_eq!(report["details"]["tags_added"], 3);
    assert_eq!(json(dir, &["search", "tag:nl::src::freq"]).as_array().unwrap().len(), 3);

    // Field edits go the same way, and only the notes that change are counted.
    let report = json(
        dir,
        &["edit", "--query", "deck:Freq", "--field", "Back=same"],
    );
    assert_eq!(report["details"]["fields_updated"], 3);
    let report = json(
        dir,
        &["edit", "--query", "deck:Freq", "--field", "Back=same"],
    );
    assert_eq!(report["details"]["fields_updated"], 0);

    cli(dir)
        .args(["edit", "--query", "deck:Freq"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to change"));
}

#[test]
fn status_reports_queue_breakdown() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    frequency_deck(dir, "Freq", &[1, 2, 3]);
    cli(dir)
        .args(["suspend", "deck:Freq", "--sort-field", "Rank", "--limit", "2"])
        .assert()
        .success();

    let report = json(dir, &["status", "--offline"]);
    assert_eq!(report["queues"]["new"], 1);
    assert_eq!(report["queues"]["suspended"], 2);

    let decks = json(dir, &["decks"]);
    let freq = decks
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "Freq")
        .unwrap();
    assert_eq!(freq["cards"], 3);
    assert_eq!(freq["new"], 1);
    assert_eq!(freq["suspended"], 2);
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
