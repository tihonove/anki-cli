//! Minimal MCP (Model Context Protocol) server over stdio.
//!
//! Speaks newline-delimited JSON-RPC 2.0, exposing the collection operations
//! as tools. Authenticate with the `anki_login` tool (or run `anki-cli login`
//! beforehand). To keep the password out of the conversation, `anki_login` reads
//! `ANKI_USERNAME` / `ANKI_PASSWORD` from the server's environment when the
//! arguments are omitted. Either way only the resulting session key is stored.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::config::{self, Config};
use crate::ops::OpReport;
use crate::select::{self, Selector, Sort};
use crate::{cards, col, decks, models, notes, sync};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// Sent with `initialize`. Twenty-eight tool descriptions do not add up to a
/// mental model, and the things that bite hardest — notes vs cards, sync
/// direction, which operations force a full upload — are exactly the ones no
/// single tool description can carry.
const INSTRUCTIONS: &str = "\
Manages a local Anki collection that syncs with AnkiWeb.

SYNC. The collection is local, like a git checkout. Start with anki_pull (or anki_sync) to \
get the current state, make changes, then anki_sync to send them back. If anki_sync returns \
result \"conflict\", the two sides diverged and cannot be merged: resolve with anki_pull \
(take the server's version, discarding local changes) or anki_push (take the local version, \
overwriting the server). Media files are a separate step: anki_sync_media.

NOTES VS CARDS. A note holds the content; its cards hold the scheduling. anki_search returns \
notes, anki_cards returns cards with queue/type/due/interval. Questions like \"what is \
currently not suspended\" or \"what will be studied next\" are card questions — anki_search \
cannot answer them, use anki_cards.

SELECTING WHAT TO ACT ON. Every card and note tool takes either `query` (Anki search syntax, \
e.g. 'deck:\"A Frequency Dictionary of Dutch\" is:suspended') or explicit `card_ids` / \
`note_ids` — one or the other, never both, and never an empty query. Narrow the selection \
with `limit`, `sort`, `sort_field` and `reverse`. `sort_field` orders by the value of a \
notetype field and compares numerically when the values are numeric, so `sort_field` plus \
`limit` is how you take \"the next 10 by Rank\" rather than an arbitrary 10. Prefer one \
query-driven call over a loop of per-id calls: these decks run to five figures.

BEFORE ANYTHING DESTRUCTIVE. Every destructive tool takes `dry_run`. Called that way it \
reports `matched` and what would happen, and changes nothing. Use it first for deck \
deletion, field removal and any bulk edit whose selection you have not verified.

SCHEMA CHANGES. anki_add_field, anki_remove_field, anki_move_field and anki_edit_template \
change the collection's schema. After any of them a normal anki_sync reports a conflict — \
the collection has to go up with anki_push instead. The tools flag this as \
`full_sync_required` in their result.";

pub async fn serve(dir_flag: Option<PathBuf>) -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
        // Requests carry an id; notifications don't and get no reply.
        let Some(id) = msg.get("id").cloned().filter(|id| !id.is_null()) else {
            continue;
        };

        let response = match method {
            "initialize" => {
                let requested = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL_VERSION);
                ok(&id, json!({
                    "protocolVersion": requested,
                    "capabilities": {"tools": {}},
                    "serverInfo": {
                        "name": "anki-cli",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "instructions": INSTRUCTIONS,
                }))
            }
            "ping" => ok(&id, json!({})),
            "tools/list" => ok(&id, json!({"tools": tool_definitions()})),
            "tools/call" => match call_tool(&dir_flag, &params).await {
                Ok(text) => ok(&id, json!({
                    "content": [{"type": "text", "text": text}],
                    "isError": false,
                })),
                Err(e) => ok(&id, json!({
                    "content": [{"type": "text", "text": format!("{e:#}")}],
                    "isError": true,
                })),
            },
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("method not found: {method}")},
            }),
        };
        writeln!(stdout, "{response}")?;
        stdout.flush()?;
    }
    Ok(())
}

fn ok(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// The schema shared by every bulk tool: point it at a query or at ids, then
/// narrow with sorting and a limit. Extra properties are merged in.
fn selector_schema(extra: Value, required: &[&str]) -> Value {
    let mut props = json!({
        "query": {"type": "string", "description": "Anki search, e.g. 'deck:\"A Frequency Dictionary of Dutch\" is:suspended'. Mutually exclusive with card_ids/note_ids."},
        "card_ids": {"type": "array", "items": {"type": "integer"}, "description": "Card ids, instead of a query"},
        "note_ids": {"type": "array", "items": {"type": "integer"}, "description": "Note ids, instead of a query: matches every card of these notes"},
        "limit": {"type": "integer", "description": "Keep only the first N after sorting"},
        "sort": {"type": "string", "enum": Sort::names(), "description": "Built-in ordering to apply before the limit"},
        "sort_field": {"type": "string", "description": "Order by the value of a notetype field, numerically when it looks numeric — e.g. \"Rank\" to take the next N words by frequency"},
        "reverse": {"type": "boolean", "default": false},
        "dry_run": {"type": "boolean", "default": false, "description": "Report what would change, without changing it"},
    });
    if let (Some(props), Some(extra)) = (props.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            props.insert(key.clone(), value.clone());
        }
    }
    json!({"type": "object", "properties": props, "required": required})
}

fn tool_definitions() -> Value {
    let fields_prop = json!({
        "type": "object",
        "description": "Field values by field name, e.g. {\"Front\": \"hola\", \"Back\": \"привет\"}",
        "additionalProperties": {"type": "string"},
    });
    let tags_prop = json!({"type": "array", "items": {"type": "string"}});
    json!([
        {
            "name": "anki_login",
            "description": "Authenticate against AnkiWeb (or a custom sync server) and store the session key in .anki/config.json. The password is exchanged for a session key and never stored. If username/password are omitted, the server's ANKI_USERNAME / ANKI_PASSWORD environment variables are used — prefer that so the password stays out of the conversation.",
            "inputSchema": {"type": "object", "properties": {
                "username": {"type": "string", "description": "AnkiWeb email; falls back to $ANKI_USERNAME"},
                "password": {"type": "string", "description": "AnkiWeb password; falls back to $ANKI_PASSWORD. Not stored."},
                "endpoint": {"type": "string", "description": "Custom sync server URL (default: AnkiWeb)"},
            }},
        },
        {
            "name": "anki_logout",
            "description": "Forget stored credentials (clears the session key from .anki/config.json).",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "anki_status",
            "description": "Collection stats and sync state: note/card totals, a new/learning/review/suspended/buried breakdown of the whole collection, whether there are unsynced local changes, and whether the server differs. Set offline=true to skip the network check.",
            "inputSchema": {"type": "object", "properties": {
                "offline": {"type": "boolean", "default": false},
            }},
        },
        {
            "name": "anki_sync",
            "description": "Two-way sync with AnkiWeb. Result 'conflict' means the collections diverged and cannot be merged: resolve with anki_pull (take server version) or anki_push (take local version).",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "anki_pull",
            "description": "Full download: replace the local collection with the server version. Refuses to discard unsynced local changes unless force=true.",
            "inputSchema": {"type": "object", "properties": {
                "force": {"type": "boolean", "default": false},
            }},
        },
        {
            "name": "anki_push",
            "description": "Full upload: replace the server collection with the local version. Destructive to remote changes — use to resolve a sync conflict in favour of the local side.",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "anki_sync_media",
            "description": "Sync media files (images, audio) with AnkiWeb: uploads locally-added media and downloads server-side changes. Merges file-by-file and never conflicts. Run after anki_sync when notes reference media files.",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "anki_add_note",
            "description": "Add a note (creates its cards). Check field names of the model with anki_list_models first if unsure.",
            "inputSchema": {"type": "object", "properties": {
                "deck": {"type": "string", "default": "Default", "description": "Deck name; created if missing. Use :: for nesting, e.g. Deutsch::A1"},
                "model": {"type": "string", "default": "Basic", "description": "Notetype name, e.g. Basic, Cloze"},
                "fields": fields_prop,
                "tags": tags_prop,
            }, "required": ["fields"]},
        },
        {
            "name": "anki_add_notes",
            "description": "Add many notes in one call (bulk). Each entry needs `fields`; its `deck`/`model`/`tags` fall back to the top-level defaults when omitted. Returns the created notes plus any per-entry failures (by index), so a bad entry doesn't block the rest. Check field names with anki_list_models first if unsure.",
            "inputSchema": {"type": "object", "properties": {
                "deck": {"type": "string", "default": "Default", "description": "Default deck for entries without their own; created if missing. Use :: for nesting"},
                "model": {"type": "string", "default": "Basic", "description": "Default notetype for entries without their own"},
                "tags": tags_prop,
                "notes": {
                    "type": "array",
                    "description": "Notes to add",
                    "items": {"type": "object", "properties": {
                        "fields": fields_prop,
                        "deck": {"type": "string"},
                        "model": {"type": "string"},
                        "tags": tags_prop,
                    }, "required": ["fields"]},
                },
            }, "required": ["notes"]},
        },
        {
            "name": "anki_search",
            "description": "Search notes with Anki's search syntax, e.g. 'deck:Spanish tag:verb hola', 'added:7', '\"exact phrase\"'. Returns full notes; use anki_cards for scheduling state.",
            "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer", "default": 50},
                "sort": {"type": "string", "enum": Sort::names()},
                "sort_field": {"type": "string", "description": "Order by the value of a notetype field, numerically when it looks numeric"},
                "reverse": {"type": "boolean", "default": false},
            }, "required": ["query"]},
        },
        {
            "name": "anki_get_note",
            "description": "Get one note by id, with fields, tags and cards.",
            "inputSchema": {"type": "object", "properties": {
                "note_id": {"type": "integer"},
            }, "required": ["note_id"]},
        },
        {
            "name": "anki_edit_note",
            "description": "Update fields and/or tags of a note.",
            "inputSchema": {"type": "object", "properties": {
                "note_id": {"type": "integer"},
                "fields": fields_prop,
                "add_tags": tags_prop,
                "remove_tags": tags_prop,
            }, "required": ["note_id"]},
        },
        {
            "name": "anki_delete_notes",
            "description": "Delete notes (and their cards) by id, or everything matching a search. DESTRUCTIVE — pass dry_run=true first to see the count.",
            "inputSchema": {"type": "object", "properties": {
                "note_ids": {"type": "array", "items": {"type": "integer"}},
                "query": {"type": "string", "description": "Delete every note matching this search, instead of listing ids"},
                "limit": {"type": "integer"},
                "dry_run": {"type": "boolean", "default": false},
            }},
        },
        {
            "name": "anki_list_decks",
            "description": "List decks with card counts and a queue breakdown: new / learning / review / suspended / buried. The five buckets are mutually exclusive and sum to the deck's card count, so this answers 'how much is still waiting to be introduced'.",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "anki_list_models",
            "description": "List notetypes (models); pass name to get one model's field names and card templates.",
            "inputSchema": {"type": "object", "properties": {
                "name": {"type": "string"},
            }},
        },
        {
            "name": "anki_cards",
            "description": "List cards (not notes) with their scheduling state. Fields: card_id, note_id, deck, template_idx, queue (new/learn/review/day_learn/preview_repeat/suspended/sched_buried/user_buried), ctype (new/learn/review/relearn), due, ivl (review interval in days), reps, lapses, ease (permille). Note that `due` means different things per queue: for a new card it is its position in the new-card queue, for a review card it is a day number, for a learning card a timestamp — compare `due` only between cards in the same queue. This is how to answer 'what is currently not suspended': anki_search returns notes and cannot.",
            "inputSchema": selector_schema(json!({}), &[]),
        },
        {
            "name": "anki_suspend",
            "description": "Suspend cards: take them out of the study queue while keeping their progress. Accepts a search query, so a whole deck can be suspended in one call.",
            "inputSchema": selector_schema(json!({}), &[]),
        },
        {
            "name": "anki_unsuspend",
            "description": "Unsuspend cards: put them back into the study queue. Only suspended cards are touched (buried cards are left alone). Combine sort_field with limit to release the next N in a deliberate order, e.g. sort_field 'Rank', limit 10.",
            "inputSchema": selector_schema(json!({}), &[]),
        },
        {
            "name": "anki_forget",
            "description": "Reset cards to new, discarding their scheduling progress. Destructive to review history-based scheduling; the review log is kept.",
            "inputSchema": selector_schema(json!({
                "restore_position": {"type": "boolean", "default": false, "description": "Put each card back at its original position in the new queue"},
                "reset_counts": {"type": "boolean", "default": false, "description": "Also zero the review and lapse counts"},
            }), &[]),
        },
        {
            "name": "anki_reposition",
            "description": "Renumber the new-card queue, deciding the order in which new cards will be introduced. The selection order is used as-is, so pairing sort_field 'Rank' with this puts a frequency deck in frequency order. Only cards still in the new queue are affected.",
            "inputSchema": selector_schema(json!({
                "start": {"type": "integer", "default": 1, "description": "Position given to the first card"},
                "step": {"type": "integer", "default": 1, "description": "Gap between consecutive positions"},
                "randomize": {"type": "boolean", "default": false, "description": "Shuffle instead of using the selection order"},
                "shift": {"type": "boolean", "default": false, "description": "Push existing cards out of the way instead of overlapping them"},
            }), &[]),
        },
        {
            "name": "anki_move_cards",
            "description": "Move cards to another deck. The destination is created if missing unless no_create is set.",
            "inputSchema": selector_schema(json!({
                "deck": {"type": "string", "description": "Destination deck name; :: nests, e.g. Deutsch::A1"},
                "no_create": {"type": "boolean", "default": false, "description": "Fail instead of creating a missing destination"},
            }), &["deck"]),
        },
        {
            "name": "anki_bulk_edit_notes",
            "description": "Apply the same field values and/or tag changes to every note matching a search — one call instead of one per note. Tag changes and field updates each run as a single transaction.",
            "inputSchema": selector_schema(json!({
                "fields": fields_prop,
                "add_tags": tags_prop,
                "remove_tags": tags_prop,
            }), &[]),
        },
        {
            "name": "anki_delete_deck",
            "description": "Delete a deck and its child decks. DESTRUCTIVE, and the two modes differ sharply: with_notes=true deletes the deck's notes as well (a note survives only if it has cards in another deck), while keep_notes=true first moves every card to move_to (default 'Default') so no note is lost. Exactly one of the two must be set. Run with dry_run=true first to see the note count at stake.",
            "inputSchema": {"type": "object", "properties": {
                "name": {"type": "string"},
                "with_notes": {"type": "boolean", "default": false, "description": "Delete the deck's notes along with it"},
                "keep_notes": {"type": "boolean", "default": false, "description": "Move the deck's cards elsewhere first, so no note is lost"},
                "move_to": {"type": "string", "default": "Default", "description": "Where keep_notes moves the cards"},
                "dry_run": {"type": "boolean", "default": false},
            }, "required": ["name"]},
        },
        {
            "name": "anki_rename_deck",
            "description": "Rename a deck. Child decks follow automatically. Use :: to move a deck under another parent. A clashing name is uniquified rather than rejected, so check the returned name.",
            "inputSchema": {"type": "object", "properties": {
                "from": {"type": "string"},
                "to": {"type": "string"},
            }, "required": ["from", "to"]},
        },
        {
            "name": "anki_add_field",
            "description": "Add a field to a notetype. Existing notes keep their values and get an empty value in the new field. SCHEMA CHANGE: afterwards a normal anki_sync reports 'conflict' — anki_push is required to upload the new schema.",
            "inputSchema": {"type": "object", "properties": {
                "model": {"type": "string"},
                "field": {"type": "string"},
                "pos": {"type": "integer", "description": "0-based position; appended at the end by default"},
            }, "required": ["model", "field"]},
        },
        {
            "name": "anki_remove_field",
            "description": "Remove a field from a notetype. DESTRUCTIVE: its value is lost on every note, and references to it are stripped from the card templates. Use dry_run=true to see how many notes hold data in it. SCHEMA CHANGE: anki_push is required afterwards.",
            "inputSchema": {"type": "object", "properties": {
                "model": {"type": "string"},
                "field": {"type": "string"},
                "dry_run": {"type": "boolean", "default": false},
            }, "required": ["model", "field"]},
        },
        {
            "name": "anki_move_field",
            "description": "Move a field to another position in a notetype, keeping its values. SCHEMA CHANGE: anki_push is required afterwards.",
            "inputSchema": {"type": "object", "properties": {
                "model": {"type": "string"},
                "field": {"type": "string"},
                "pos": {"type": "integer", "description": "New 0-based position"},
            }, "required": ["model", "field", "pos"]},
        },
        {
            "name": "anki_edit_template",
            "description": "Replace a card template's front and/or back format (Anki template syntax, e.g. '{{Front}}'). Changing the front triggers card regeneration for the whole notetype and can create new cards. SCHEMA CHANGE: anki_push is required afterwards.",
            "inputSchema": {"type": "object", "properties": {
                "model": {"type": "string"},
                "card": {"type": "integer", "default": 1, "description": "Which card template, 1-based (Anki's \"Card 1\")"},
                "front": {"type": "string", "description": "New front template"},
                "back": {"type": "string", "description": "New back template"},
            }, "required": ["model"]},
        },
    ])
}

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

/// A credential from the environment, ignoring an unset-or-empty variable.
fn env_cred(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|v| !v.is_empty())
}

fn tags_arg(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn fields_arg(args: &Value, key: &str) -> Vec<(String, String)> {
    args.get(key)
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn ids_arg(args: &Value, key: &str) -> Vec<i64> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

fn bool_arg(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn usize_arg(args: &Value, key: &str) -> Option<usize> {
    args.get(key).and_then(Value::as_u64).map(|n| n as usize)
}

/// The selector every bulk tool shares.
fn selector_arg(args: &Value) -> Result<Selector> {
    let sort = match str_arg(args, "sort") {
        Some(s) => Some(s.parse::<Sort>()?),
        None => None,
    };
    Ok(Selector {
        query: str_arg(args, "query"),
        ids: ids_arg(args, "card_ids"),
        nids: ids_arg(args, "note_ids"),
        sort,
        sort_field: str_arg(args, "sort_field"),
        reverse: bool_arg(args, "reverse"),
        limit: usize_arg(args, "limit"),
    })
}

fn pretty<T: serde::Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string_pretty(value)?)
}

async fn call_tool(dir_flag: &Option<PathBuf>, params: &Value) -> Result<String> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing tool name"))?;
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

    let dir = config::resolve_dir(dir_flag.clone())?;
    let mut cfg = Config::load(&dir)?;

    match name {
        "anki_login" => {
            let username = str_arg(&args, "username")
                .or_else(|| env_cred("ANKI_USERNAME"))
                .ok_or_else(|| anyhow!("missing username (pass it or set ANKI_USERNAME)"))?;
            let password = str_arg(&args, "password")
                .or_else(|| env_cred("ANKI_PASSWORD"))
                .ok_or_else(|| anyhow!("missing password (pass it or set ANKI_PASSWORD)"))?;
            let endpoint = str_arg(&args, "endpoint");
            sync::login(&dir, &mut cfg, &username, &password, endpoint).await?;
            pretty(&json!({"logged_in": username}))
        }
        "anki_logout" => {
            cfg.hkey = None;
            cfg.save(&dir)?;
            pretty(&json!({"logged_out": true}))
        }
        "anki_status" => {
            let offline = args.get("offline").and_then(Value::as_bool).unwrap_or(false);
            pretty(&sync::status(&dir, &cfg, offline).await?)
        }
        "anki_sync" => pretty(&sync::normal_sync(&dir, &mut cfg).await?),
        "anki_pull" => {
            let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
            sync::pull(&dir, &mut cfg, force).await?;
            let report = sync::status(&dir, &cfg, true).await?;
            pretty(&json!({"pulled": true, "notes": report.notes, "cards": report.cards}))
        }
        "anki_push" => {
            sync::push(&dir, &mut cfg).await?;
            pretty(&json!({"pushed": true}))
        }
        "anki_sync_media" => pretty(&sync::sync_media(&dir, &mut cfg).await?),
        "anki_add_note" => {
            let deck = str_arg(&args, "deck").unwrap_or_else(|| "Default".into());
            let model = str_arg(&args, "model").unwrap_or_else(|| "Basic".into());
            let fields = fields_arg(&args, "fields");
            if fields.is_empty() {
                return Err(anyhow!("fields must be a non-empty object of name→value"));
            }
            let tags = tags_arg(&args, "tags");
            let mut col = col::open_collection(&dir)?;
            pretty(&notes::add_note(&mut col, &deck, &model, &[], &fields, &tags)?)
        }
        "anki_add_notes" => {
            let default_deck = str_arg(&args, "deck").unwrap_or_else(|| "Default".into());
            let default_model = str_arg(&args, "model").unwrap_or_else(|| "Basic".into());
            let default_tags = tags_arg(&args, "tags");
            let items = args
                .get("notes")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("notes must be an array of note objects"))?;
            if items.is_empty() {
                return Err(anyhow!("notes must be a non-empty array"));
            }
            let mut col = col::open_collection(&dir)?;
            let mut added = Vec::new();
            let mut failed = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let fields = fields_arg(item, "fields");
                if fields.is_empty() {
                    failed.push(json!({"index": i, "error": "fields must be a non-empty object of name→value"}));
                    continue;
                }
                let deck = str_arg(item, "deck").unwrap_or_else(|| default_deck.clone());
                let model = str_arg(item, "model").unwrap_or_else(|| default_model.clone());
                let mut tags = tags_arg(item, "tags");
                if tags.is_empty() {
                    tags = default_tags.clone();
                }
                match notes::add_note(&mut col, &deck, &model, &[], &fields, &tags) {
                    Ok(info) => added.push(info),
                    Err(e) => failed.push(json!({"index": i, "error": format!("{e:#}")})),
                }
            }
            pretty(&json!({
                "added": serde_json::to_value(&added)?,
                "failed": Value::Array(failed),
            }))
        }
        "anki_search" => {
            let query = str_arg(&args, "query").ok_or_else(|| anyhow!("missing query"))?;
            let sel = Selector {
                query: Some(query),
                limit: Some(usize_arg(&args, "limit").unwrap_or(50)),
                ..selector_arg(&args)?
            };
            let mut col = col::open_collection(&dir)?;
            let nids = select::resolve_notes(&mut col, &sel)?;
            pretty(&notes::notes_info(&mut col, &nids)?)
        }
        "anki_get_note" => {
            let nid = args
                .get("note_id")
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow!("missing note_id"))?;
            let mut col = col::open_collection(&dir)?;
            pretty(&notes::note_info(&mut col, anki::notes::NoteId(nid))?)
        }
        "anki_edit_note" => {
            let nid = args
                .get("note_id")
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow!("missing note_id"))?;
            let fields = fields_arg(&args, "fields");
            let add = tags_arg(&args, "add_tags");
            let remove = tags_arg(&args, "remove_tags");
            if fields.is_empty() && add.is_empty() && remove.is_empty() {
                return Err(anyhow!("nothing to change: pass fields, add_tags or remove_tags"));
            }
            let mut col = col::open_collection(&dir)?;
            pretty(&notes::edit_note(&mut col, nid, &fields, &add, &remove)?)
        }
        "anki_delete_notes" => {
            let dry_run = bool_arg(&args, "dry_run");
            let sel = Selector {
                query: str_arg(&args, "query"),
                ids: ids_arg(&args, "note_ids"),
                limit: usize_arg(&args, "limit"),
                ..Default::default()
            };
            let mut col = col::open_collection(&dir)?;
            let nids = select::resolve_notes(&mut col, &sel)?;
            let mut report = OpReport::new("delete_notes", dry_run, nids.len()).with_notes(&nids);
            if !dry_run {
                let removed = notes::remove_notes(&mut col, &nids)?;
                report = report
                    .changed(nids.len())
                    .with_details(json!({"removed_cards": removed}));
            }
            pretty(&report)
        }
        "anki_list_decks" => {
            let mut col = col::open_collection(&dir)?;
            pretty(&decks::list(&mut col)?)
        }
        "anki_list_models" => {
            let mut col = col::open_collection(&dir)?;
            match str_arg(&args, "name") {
                Some(name) => pretty(&models::info(models::get(&mut col, &name)?.as_ref())),
                None => {
                    let names = col.storage.get_all_notetype_names()?;
                    let list: Vec<_> = names
                        .iter()
                        .map(|(id, n)| json!({"id": id.0, "name": n}))
                        .collect();
                    pretty(&list)
                }
            }
        }
        "anki_cards" => {
            let sel = selector_arg(&args)?;
            let mut col = col::open_collection(&dir)?;
            let cids = select::resolve_cards(&mut col, &sel)?;
            pretty(&cards::card_rows(&mut col, &cids)?)
        }
        "anki_suspend" => {
            let (sel, dry_run) = (selector_arg(&args)?, bool_arg(&args, "dry_run"));
            let mut col = col::open_collection(&dir)?;
            let cids = select::resolve_cards(&mut col, &sel)?;
            let mut report = OpReport::new("suspend", dry_run, cids.len()).with_cards(&cids);
            if !dry_run {
                report = report.changed(cards::suspend(&mut col, &cids)?);
            }
            pretty(&report)
        }
        "anki_unsuspend" => {
            let (sel, dry_run) = (selector_arg(&args)?, bool_arg(&args, "dry_run"));
            let mut col = col::open_collection(&dir)?;
            let cids = select::resolve_cards(&mut col, &sel)?;
            let mut report = OpReport::new("unsuspend", dry_run, cids.len()).with_cards(&cids);
            if dry_run {
                let suspended = cards::suspended_among(&mut col, &cids)?;
                report = report.with_details(json!({"currently_suspended": suspended.len()}));
            } else {
                report = report.changed(cards::unsuspend(&mut col, &cids)?);
            }
            pretty(&report)
        }
        "anki_forget" => {
            let (sel, dry_run) = (selector_arg(&args)?, bool_arg(&args, "dry_run"));
            let mut col = col::open_collection(&dir)?;
            let cids = select::resolve_cards(&mut col, &sel)?;
            let mut report = OpReport::new("forget", dry_run, cids.len()).with_cards(&cids);
            if !dry_run {
                let changed = cards::forget(
                    &mut col,
                    &cids,
                    bool_arg(&args, "restore_position"),
                    bool_arg(&args, "reset_counts"),
                )?;
                report = report.changed(changed);
            }
            pretty(&report)
        }
        "anki_reposition" => {
            let (sel, dry_run) = (selector_arg(&args)?, bool_arg(&args, "dry_run"));
            let mut col = col::open_collection(&dir)?;
            let cids = select::resolve_cards(&mut col, &sel)?;
            let mut report = OpReport::new("reposition", dry_run, cids.len()).with_cards(&cids);
            if !dry_run {
                let changed = cards::reposition(
                    &mut col,
                    &cids,
                    usize_arg(&args, "start").unwrap_or(1) as u32,
                    usize_arg(&args, "step").unwrap_or(1) as u32,
                    bool_arg(&args, "randomize"),
                    bool_arg(&args, "shift"),
                )?;
                report = report.changed(changed);
            }
            pretty(&report)
        }
        "anki_move_cards" => {
            let (sel, dry_run) = (selector_arg(&args)?, bool_arg(&args, "dry_run"));
            let deck = str_arg(&args, "deck").ok_or_else(|| anyhow!("missing deck"))?;
            let mut col = col::open_collection(&dir)?;
            let cids = select::resolve_cards(&mut col, &sel)?;
            let mut report = OpReport::new("move_cards", dry_run, cids.len()).with_cards(&cids);
            if dry_run {
                report = report.with_details(json!({"deck": deck}));
            } else {
                let (changed, name) =
                    cards::move_to_deck(&mut col, &cids, &deck, !bool_arg(&args, "no_create"))?;
                report = report.changed(changed).with_details(json!({"deck": name}));
            }
            pretty(&report)
        }
        "anki_bulk_edit_notes" => {
            let (sel, dry_run) = (selector_arg(&args)?, bool_arg(&args, "dry_run"));
            let fields = fields_arg(&args, "fields");
            let add = tags_arg(&args, "add_tags");
            let remove = tags_arg(&args, "remove_tags");
            if fields.is_empty() && add.is_empty() && remove.is_empty() {
                return Err(anyhow!("nothing to change: pass fields, add_tags or remove_tags"));
            }
            let mut col = col::open_collection(&dir)?;
            let nids = select::resolve_notes(&mut col, &sel)?;
            let mut report = OpReport::new("bulk_edit", dry_run, nids.len()).with_notes(&nids);
            if !dry_run {
                let out = notes::edit_notes(&mut col, &nids, &fields, &add, &remove)?;
                report = report
                    .changed(out.changed())
                    .with_details(serde_json::to_value(&out)?);
            }
            pretty(&report)
        }
        "anki_delete_deck" => {
            let name = str_arg(&args, "name").ok_or_else(|| anyhow!("missing name"))?;
            let with_notes = bool_arg(&args, "with_notes");
            let keep_notes = bool_arg(&args, "keep_notes");
            let dry_run = bool_arg(&args, "dry_run");
            let move_to = str_arg(&args, "move_to").unwrap_or_else(|| "Default".into());
            if with_notes == keep_notes {
                return Err(anyhow!(
                    "set exactly one of with_notes (deletes the deck's notes too) or \
                     keep_notes (moves their cards to move_to and deletes only the deck)"
                ));
            }
            let mut col = col::open_collection(&dir)?;
            let preview = decks::removal_preview(&mut col, &name)?;
            let mut report = OpReport::new("delete_deck", dry_run, preview.cards).with_details(
                json!({
                    "deck": name,
                    "cards": preview.cards,
                    "notes_deleted": if with_notes { preview.notes_deleted } else { 0 },
                    "notes_moved_to": if keep_notes { Some(move_to.clone()) } else { None },
                }),
            );
            if !dry_run {
                let out = decks::remove(&mut col, &name, keep_notes, &move_to)?;
                report = report.changed(out.cards_affected()).with_details(json!({
                    "deck": name,
                    "cards_moved": out.cards_moved,
                    "cards_removed": out.cards_removed,
                    "notes_deleted": if with_notes { preview.notes_deleted } else { 0 },
                    "notes_moved_to": out.moved_to,
                }));
            }
            pretty(&report)
        }
        "anki_rename_deck" => {
            let from = str_arg(&args, "from").ok_or_else(|| anyhow!("missing from"))?;
            let to = str_arg(&args, "to").ok_or_else(|| anyhow!("missing to"))?;
            let mut col = col::open_collection(&dir)?;
            let name = decks::rename(&mut col, &from, &to)?;
            pretty(
                &OpReport::new("rename_deck", false, 1)
                    .changed(1)
                    .with_details(json!({"from": from, "to": name})),
            )
        }
        "anki_add_field" => {
            let model = str_arg(&args, "model").ok_or_else(|| anyhow!("missing model"))?;
            let field = str_arg(&args, "field").ok_or_else(|| anyhow!("missing field"))?;
            let mut col = col::open_collection(&dir)?;
            let info = models::add_field(&mut col, &model, &field, usize_arg(&args, "pos"))?;
            pretty(
                &OpReport::new("add_field", false, 1)
                    .changed(1)
                    .needs_full_sync()
                    .with_details(serde_json::to_value(&info)?),
            )
        }
        "anki_remove_field" => {
            let model = str_arg(&args, "model").ok_or_else(|| anyhow!("missing model"))?;
            let field = str_arg(&args, "field").ok_or_else(|| anyhow!("missing field"))?;
            let dry_run = bool_arg(&args, "dry_run");
            let mut col = col::open_collection(&dir)?;
            let losing = models::notes_with_value(&mut col, &model, &field)?;
            let mut report = OpReport::new("remove_field", dry_run, 1)
                .needs_full_sync()
                .with_details(json!({
                    "model": model, "field": field, "notes_losing_data": losing,
                }));
            if !dry_run {
                let info = models::remove_field(&mut col, &model, &field)?;
                report = report.changed(1).with_details(json!({
                    "model": info.name,
                    "fields": info.fields,
                    "notes_losing_data": losing,
                }));
            }
            pretty(&report)
        }
        "anki_move_field" => {
            let model = str_arg(&args, "model").ok_or_else(|| anyhow!("missing model"))?;
            let field = str_arg(&args, "field").ok_or_else(|| anyhow!("missing field"))?;
            let pos = usize_arg(&args, "pos").ok_or_else(|| anyhow!("missing pos"))?;
            let mut col = col::open_collection(&dir)?;
            let info = models::move_field(&mut col, &model, &field, pos)?;
            pretty(
                &OpReport::new("move_field", false, 1)
                    .changed(1)
                    .needs_full_sync()
                    .with_details(serde_json::to_value(&info)?),
            )
        }
        "anki_edit_template" => {
            let model = str_arg(&args, "model").ok_or_else(|| anyhow!("missing model"))?;
            let card = usize_arg(&args, "card").unwrap_or(1);
            let mut col = col::open_collection(&dir)?;
            let info = models::edit_template(
                &mut col,
                &model,
                card,
                str_arg(&args, "front"),
                str_arg(&args, "back"),
            )?;
            pretty(
                &OpReport::new("edit_template", false, 1)
                    .changed(1)
                    .needs_full_sync()
                    .with_details(serde_json::to_value(&info)?),
            )
        }
        other => Err(anyhow!("unknown tool: {other}")),
    }
}
