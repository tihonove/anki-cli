//! Minimal MCP (Model Context Protocol) server over stdio.
//!
//! Speaks newline-delimited JSON-RPC 2.0, exposing the collection operations
//! as tools. Run `anki-cli login` in the collection directory once beforehand;
//! the server never sees the password.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::config::{self, Config};
use crate::{col, notes, sync};

const PROTOCOL_VERSION: &str = "2024-11-05";

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

fn tool_definitions() -> Value {
    let fields_prop = json!({
        "type": "object",
        "description": "Field values by field name, e.g. {\"Front\": \"hola\", \"Back\": \"привет\"}",
        "additionalProperties": {"type": "string"},
    });
    let tags_prop = json!({"type": "array", "items": {"type": "string"}});
    json!([
        {
            "name": "anki_status",
            "description": "Collection stats and sync state (local changes; whether the server differs). Set offline=true to skip the network check.",
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
            "name": "anki_search",
            "description": "Search notes with Anki's search syntax, e.g. 'deck:Spanish tag:verb hola', 'added:7', '\"exact phrase\"'. Returns full notes.",
            "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer", "default": 50},
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
            "description": "Delete notes (and their cards) by id.",
            "inputSchema": {"type": "object", "properties": {
                "note_ids": {"type": "array", "items": {"type": "integer"}},
            }, "required": ["note_ids"]},
        },
        {
            "name": "anki_list_decks",
            "description": "List decks with card counts.",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "anki_list_models",
            "description": "List notetypes (models); pass name to get one model's field names.",
            "inputSchema": {"type": "object", "properties": {
                "name": {"type": "string"},
            }},
        },
    ])
}

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
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
        "anki_search" => {
            let query = str_arg(&args, "query").ok_or_else(|| anyhow!("missing query"))?;
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
            let mut col = col::open_collection(&dir)?;
            pretty(&notes::search_notes(&mut col, &query, limit)?)
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
            let ids: Vec<i64> = args
                .get("note_ids")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default();
            if ids.is_empty() {
                return Err(anyhow!("note_ids must be a non-empty array of integers"));
            }
            let mut col = col::open_collection(&dir)?;
            let removed = notes::remove_notes(&mut col, &ids)?;
            pretty(&json!({"removed_cards": removed, "note_ids": ids}))
        }
        "anki_list_decks" => {
            let mut col = col::open_collection(&dir)?;
            let names = col.get_all_deck_names(false)?;
            let mut out = Vec::new();
            for (id, name) in names {
                let count = col
                    .search_cards(
                        format!("deck:\"{name}\"").as_str(),
                        anki::search::SortMode::NoOrder,
                    )?
                    .len();
                out.push(json!({"id": id.0, "name": name, "cards": count}));
            }
            pretty(&out)
        }
        "anki_list_models" => {
            let mut col = col::open_collection(&dir)?;
            match str_arg(&args, "name") {
                Some(name) => {
                    let nt = col
                        .get_notetype_by_name(&name)?
                        .ok_or_else(|| anyhow!("no notetype named '{name}'"))?;
                    let fields: Vec<&str> = nt.fields.iter().map(|f| f.name.as_str()).collect();
                    pretty(&json!({"name": nt.name, "fields": fields}))
                }
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
        other => Err(anyhow!("unknown tool: {other}")),
    }
}
