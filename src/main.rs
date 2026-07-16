mod col;
mod config;
mod notes;
mod sync;

use std::path::PathBuf;
use std::process::ExitCode;

use anki::search::SortMode;
use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::notes::NoteInfo;

#[derive(Parser)]
#[command(
    name = "anki-cli",
    version,
    about = "Git-like CLI for Anki: keep a local collection, edit it, sync with AnkiWeb"
)]
struct Cli {
    /// Data directory (collection + config). Defaults to $ANKI_CLI_HOME or ~/.local/share/anki-cli
    #[arg(long, global = true)]
    dir: Option<PathBuf>,

    /// Output machine-readable JSON
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start a collection here: create ./.anki (like git init)
    Init,
    /// Authenticate against AnkiWeb (or a custom sync server) and store the session key
    Login {
        #[arg(short, long, env = "ANKI_USERNAME")]
        username: String,
        #[arg(short, long, env = "ANKI_PASSWORD")]
        password: String,
        /// Custom sync server URL (default: AnkiWeb)
        #[arg(long)]
        endpoint: Option<String>,
    },
    /// Forget stored credentials
    Logout,
    /// Show local collection stats and sync state
    Status {
        /// Skip the network round-trip to the sync server
        #[arg(long)]
        offline: bool,
    },
    /// Two-way sync with the server (like git pull+push). Exits 2 on conflict.
    Sync,
    /// Replace the local collection with the server version (full download)
    Pull {
        /// Proceed even if local unsynced changes would be discarded
        #[arg(long)]
        force: bool,
    },
    /// Replace the server collection with the local version (full upload)
    Push,
    /// Add a note. Field values positionally in notetype order, or via --field
    Add {
        /// Deck to add the card(s) to (created if missing)
        #[arg(short, long, default_value = "Default")]
        deck: String,
        /// Notetype (model) name
        #[arg(short, long, default_value = "Basic")]
        model: String,
        /// Field values in notetype order (e.g. front back)
        values: Vec<String>,
        /// Set a field by name: --field Front="Hello"
        #[arg(short, long = "field", value_name = "NAME=VALUE")]
        fields: Vec<String>,
        /// Tags, comma- or space-separated
        #[arg(short, long, default_value = "")]
        tags: String,
    },
    /// Search notes using Anki's search syntax (e.g. 'deck:Spanish tag:verb hola')
    Search {
        query: String,
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
    },
    /// Show a note in full
    Show { note_id: i64 },
    /// Edit a note's fields or tags
    Edit {
        note_id: i64,
        /// Set a field by name: --field Back="New value"
        #[arg(short, long = "field", value_name = "NAME=VALUE")]
        fields: Vec<String>,
        /// Tags to add (comma- or space-separated)
        #[arg(long, default_value = "")]
        add_tags: String,
        /// Tags to remove (comma- or space-separated)
        #[arg(long, default_value = "")]
        remove_tags: String,
    },
    /// Delete notes (and their cards)
    Rm { note_ids: Vec<i64> },
    /// List decks
    Decks,
    /// List notetypes (models), or show fields of one
    Models {
        /// Show field names of this notetype
        name: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    match rt.block_on(run(&cli)) {
        Ok(code) => code,
        Err(e) => {
            if cli.json {
                let obj = serde_json::json!({"error": format!("{e:#}")});
                eprintln!("{obj}");
            } else {
                eprintln!("error: {e:#}");
            }
            ExitCode::from(1)
        }
    }
}

fn print_json<T: serde::Serialize>(value: &T) {
    println!("{}", serde_json::to_string_pretty(value).expect("serializing output"));
}

fn oneline(text: &str, max: usize) -> String {
    let stripped = anki::text::strip_html(text).replace('\n', " ");
    let mut s = stripped.trim().to_string();
    if s.chars().count() > max {
        s = s.chars().take(max - 1).collect::<String>() + "…";
    }
    s
}

fn print_note_brief(n: &NoteInfo) {
    let fields = n
        .fields
        .iter()
        .map(|f| oneline(&f.value, 40))
        .collect::<Vec<_>>()
        .join(" | ");
    let tags = if n.tags.is_empty() {
        String::new()
    } else {
        format!("  [{}]", n.tags.join(", "))
    };
    let deck = n
        .cards
        .first()
        .map(|c| format!("  ({})", c.deck))
        .unwrap_or_default();
    println!("{}  {}{}{}", n.note_id, fields, tags, deck);
}

fn print_note_full(n: &NoteInfo) {
    println!("note id: {}", n.note_id);
    println!("model:   {}", n.model);
    for f in &n.fields {
        println!("{}: {}", f.name, f.value);
    }
    if !n.tags.is_empty() {
        println!("tags:    {}", n.tags.join(", "));
    }
    for c in &n.cards {
        println!("card:    {} in deck '{}'", c.card_id, c.deck);
    }
}

async fn run(cli: &Cli) -> Result<ExitCode> {
    if let Command::Init = &cli.command {
        let dir = match &cli.dir {
            Some(dir) => config::init_dir_at(dir)?,
            None => config::init_dir(&std::env::current_dir()?)?,
        };
        if cli.json {
            print_json(&serde_json::json!({"initialized": dir}));
        } else {
            println!("Initialized empty Anki collection dir at {}.", dir.display());
            println!("Next: `anki-cli login -u <email> -p <password>`, then `anki-cli pull`.");
        }
        return Ok(ExitCode::SUCCESS);
    }

    let dir = config::resolve_dir(cli.dir.clone())?;
    let mut cfg = Config::load(&dir)?;

    match &cli.command {
        Command::Init => unreachable!("handled above"),
        Command::Login {
            username,
            password,
            endpoint,
        } => {
            sync::login(&dir, &mut cfg, username, password, endpoint.clone()).await?;
            if cli.json {
                print_json(&serde_json::json!({"logged_in": username}));
            } else {
                println!("Logged in as {username}.");
            }
        }
        Command::Logout => {
            cfg.hkey = None;
            cfg.save(&dir)?;
            if cli.json {
                print_json(&serde_json::json!({"logged_out": true}));
            } else {
                println!("Logged out.");
            }
        }
        Command::Status { offline } => {
            let report = sync::status(&dir, &cfg, *offline).await?;
            if cli.json {
                print_json(&report);
            } else {
                println!("collection: {} notes, {} cards", report.notes, report.cards);
                println!(
                    "local:      {}",
                    if report.local_changes {
                        "changes not yet synced"
                    } else {
                        "clean"
                    }
                );
                let remote = match report.remote.as_str() {
                    "up_to_date" => "up to date with server".to_string(),
                    "sync_needed" => "differs from server — run `anki-cli sync`".to_string(),
                    "conflict" => {
                        "diverged from server — run `anki-cli sync` for options".to_string()
                    }
                    other => other.to_string(),
                };
                println!("remote:     {remote}");
                if let Some(msg) = &report.server_message {
                    println!("server says: {msg}");
                }
            }
        }
        Command::Sync => {
            let report = sync::normal_sync(&dir, &mut cfg).await?;
            if cli.json {
                print_json(&report);
            } else {
                match report.result.as_str() {
                    "up_to_date" => println!("Already up to date."),
                    "synced" => println!("Sync complete."),
                    "conflict" => {
                        println!("Conflict: {}", report.hint.as_deref().unwrap_or_default())
                    }
                    _ => {}
                }
                if let Some(msg) = &report.server_message {
                    println!("server says: {msg}");
                }
            }
            if report.result == "conflict" {
                return Ok(ExitCode::from(2));
            }
        }
        Command::Pull { force } => {
            sync::pull(&dir, &mut cfg, *force).await?;
            let report = sync::status(&dir, &cfg, true).await?;
            if cli.json {
                print_json(&serde_json::json!({
                    "pulled": true, "notes": report.notes, "cards": report.cards
                }));
            } else {
                println!(
                    "Downloaded collection from server: {} notes, {} cards.",
                    report.notes, report.cards
                );
            }
        }
        Command::Push => {
            sync::push(&dir, &mut cfg).await?;
            if cli.json {
                print_json(&serde_json::json!({"pushed": true}));
            } else {
                println!("Uploaded local collection to server.");
            }
        }
        Command::Add {
            deck,
            model,
            values,
            fields,
            tags,
        } => {
            let named = notes::parse_field_args(fields)?;
            let tags = notes::parse_tags(tags);
            let mut col = col::open_collection(&dir)?;
            let info = notes::add_note(&mut col, deck, model, values, &named, &tags)?;
            if cli.json {
                print_json(&info);
            } else {
                println!("Added note {} to deck '{}'.", info.note_id, deck);
            }
        }
        Command::Search { query, limit } => {
            let mut col = col::open_collection(&dir)?;
            let results = notes::search_notes(&mut col, query, *limit)?;
            if cli.json {
                print_json(&results);
            } else if results.is_empty() {
                println!("No notes found.");
            } else {
                for n in &results {
                    print_note_brief(n);
                }
            }
        }
        Command::Show { note_id } => {
            let mut col = col::open_collection(&dir)?;
            let info = notes::note_info(&mut col, anki::notes::NoteId(*note_id))?;
            if cli.json {
                print_json(&info);
            } else {
                print_note_full(&info);
            }
        }
        Command::Edit {
            note_id,
            fields,
            add_tags,
            remove_tags,
        } => {
            let named = notes::parse_field_args(fields)?;
            let add = notes::parse_tags(add_tags);
            let remove = notes::parse_tags(remove_tags);
            if named.is_empty() && add.is_empty() && remove.is_empty() {
                return Err(anyhow!("nothing to change: pass --field, --add-tags or --remove-tags"));
            }
            let mut col = col::open_collection(&dir)?;
            let info = notes::edit_note(&mut col, *note_id, &named, &add, &remove)?;
            if cli.json {
                print_json(&info);
            } else {
                print_note_full(&info);
            }
        }
        Command::Rm { note_ids } => {
            if note_ids.is_empty() {
                return Err(anyhow!("pass at least one note id"));
            }
            let mut col = col::open_collection(&dir)?;
            let removed = notes::remove_notes(&mut col, note_ids)?;
            if cli.json {
                print_json(&serde_json::json!({"removed_cards": removed, "note_ids": note_ids}));
            } else {
                println!("Removed {} note(s).", note_ids.len());
            }
        }
        Command::Decks => {
            let mut col = col::open_collection(&dir)?;
            let names = col.get_all_deck_names(false)?;
            let mut out = Vec::new();
            for (id, name) in names {
                let count = col
                    .search_cards(format!("deck:\"{name}\"").as_str(), SortMode::NoOrder)?
                    .len();
                out.push(serde_json::json!({"id": id.0, "name": name, "cards": count}));
            }
            if cli.json {
                print_json(&out);
            } else {
                for deck in &out {
                    println!(
                        "{}  ({} cards)",
                        deck["name"].as_str().unwrap_or_default(),
                        deck["cards"]
                    );
                }
            }
        }
        Command::Models { name } => {
            let mut col = col::open_collection(&dir)?;
            match name {
                Some(name) => {
                    let nt = col
                        .get_notetype_by_name(name)?
                        .ok_or_else(|| anyhow!("no notetype named '{name}'"))?;
                    let fields: Vec<&str> = nt.fields.iter().map(|f| f.name.as_str()).collect();
                    if cli.json {
                        print_json(&serde_json::json!({
                            "name": nt.name, "fields": fields,
                            "templates": nt.templates.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
                        }));
                    } else {
                        println!("notetype: {}", nt.name);
                        println!("fields:   {}", fields.join(", "));
                    }
                }
                None => {
                    let names = col.storage.get_all_notetype_names()?;
                    if cli.json {
                        let list: Vec<_> = names
                            .iter()
                            .map(|(id, n)| serde_json::json!({"id": id.0, "name": n}))
                            .collect();
                        print_json(&list);
                    } else {
                        for (_, n) in names {
                            println!("{n}");
                        }
                    }
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
