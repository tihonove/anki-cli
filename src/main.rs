mod cards;
mod col;
mod config;
mod decks;
mod mcp;
mod models;
mod notes;
mod ops;
mod select;
mod sync;

use std::path::PathBuf;
use std::process::ExitCode;

use anki::notes::NoteId;
use anyhow::{anyhow, bail, Result};
use clap::{Args, Parser, Subcommand};

use crate::config::Config;
use crate::notes::NoteInfo;
use crate::ops::OpReport;
use crate::select::{Selector, Sort};

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

/// What a bulk command operates on: a search query, or explicit ids.
#[derive(Args, Clone, Debug)]
struct SelectArgs {
    /// Anki search query, e.g. 'deck:"A Frequency Dictionary of Dutch" is:suspended'
    query: Option<String>,
    /// Card ids instead of a query
    #[arg(long, num_args = 1.., value_delimiter = ',', conflicts_with = "query")]
    ids: Vec<i64>,
    /// Note ids instead of a query: every card of these notes
    #[arg(long, num_args = 1.., value_delimiter = ',', conflicts_with = "query")]
    nids: Vec<i64>,
    /// Take only the first N after sorting
    #[arg(long)]
    limit: Option<usize>,
    /// Order by a built-in column
    #[arg(long, value_enum)]
    sort: Option<Sort>,
    /// Order by the value of a notetype field (numeric when it looks numeric), e.g. Rank
    #[arg(long, value_name = "NAME")]
    sort_field: Option<String>,
    /// Reverse the ordering
    #[arg(long)]
    reverse: bool,
}

impl From<&SelectArgs> for Selector {
    fn from(a: &SelectArgs) -> Self {
        Selector {
            query: a.query.clone(),
            ids: a.ids.clone(),
            nids: a.nids.clone(),
            sort: a.sort,
            sort_field: a.sort_field.clone(),
            reverse: a.reverse,
            limit: a.limit,
        }
    }
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
    /// Sync media files (images, audio) with the server. Merges file-by-file; never conflicts
    SyncMedia,
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
        /// Order by a built-in column
        #[arg(long, value_enum)]
        sort: Option<Sort>,
        /// Order by the value of a notetype field, e.g. Rank
        #[arg(long, value_name = "NAME")]
        sort_field: Option<String>,
        /// Reverse the ordering
        #[arg(long)]
        reverse: bool,
    },
    /// List cards (not notes) with their scheduling state: queue, type, due, interval
    Cards {
        #[command(flatten)]
        sel: SelectArgs,
    },
    /// Show a note in full
    Show { note_id: i64 },
    /// Edit one note by id, or many at once with --query / --ids
    Edit {
        note_id: Option<i64>,
        /// Edit every note matching this search instead of a single id
        #[arg(long, conflicts_with = "note_id")]
        query: Option<String>,
        /// Note ids to edit
        #[arg(long, num_args = 1.., value_delimiter = ',', conflicts_with_all = ["note_id", "query"])]
        ids: Vec<i64>,
        /// Take only the first N after sorting
        #[arg(long)]
        limit: Option<usize>,
        /// Order by a built-in column
        #[arg(long, value_enum)]
        sort: Option<Sort>,
        /// Order by the value of a notetype field, e.g. Rank
        #[arg(long, value_name = "NAME")]
        sort_field: Option<String>,
        /// Reverse the ordering
        #[arg(long)]
        reverse: bool,
        /// Set a field by name: --field Back="New value"
        #[arg(short, long = "field", value_name = "NAME=VALUE")]
        fields: Vec<String>,
        /// Tags to add (comma- or space-separated)
        #[arg(long, default_value = "")]
        add_tags: String,
        /// Tags to remove (comma- or space-separated)
        #[arg(long, default_value = "")]
        remove_tags: String,
        /// Report what would change, without changing it
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete notes (and their cards) by id or search
    Rm {
        note_ids: Vec<i64>,
        /// Delete every note matching this search
        #[arg(long, conflicts_with = "note_ids")]
        query: Option<String>,
        /// Take only the first N
        #[arg(long)]
        limit: Option<usize>,
        /// Report what would be deleted, without deleting it
        #[arg(long)]
        dry_run: bool,
    },
    /// Suspend cards: take them out of the study queue, keeping their progress
    Suspend {
        #[command(flatten)]
        sel: SelectArgs,
        /// Report what would change, without changing it
        #[arg(long)]
        dry_run: bool,
    },
    /// Unsuspend cards: put them back into the study queue
    Unsuspend {
        #[command(flatten)]
        sel: SelectArgs,
        /// Report what would change, without changing it
        #[arg(long)]
        dry_run: bool,
    },
    /// Reset cards to new, discarding their scheduling progress
    Forget {
        #[command(flatten)]
        sel: SelectArgs,
        /// Put each card back at its original position in the new queue
        #[arg(long)]
        restore_position: bool,
        /// Also zero the review and lapse counts
        #[arg(long)]
        reset_counts: bool,
        /// Report what would change, without changing it
        #[arg(long)]
        dry_run: bool,
    },
    /// Renumber the new-card queue. Combine with --sort-field to order by e.g. Rank
    Reposition {
        #[command(flatten)]
        sel: SelectArgs,
        /// Position given to the first card
        #[arg(long, default_value_t = 1)]
        start: u32,
        /// Gap between consecutive positions
        #[arg(long, default_value_t = 1)]
        step: u32,
        /// Shuffle instead of using the selection order
        #[arg(long)]
        randomize: bool,
        /// Push existing cards out of the way instead of overlapping them
        #[arg(long)]
        shift: bool,
        /// Report what would change, without changing it
        #[arg(long)]
        dry_run: bool,
    },
    /// Move cards to another deck
    Mv {
        #[command(flatten)]
        sel: SelectArgs,
        /// Destination deck (created unless --no-create)
        #[arg(short, long)]
        deck: String,
        /// Fail if the destination deck does not exist
        #[arg(long)]
        no_create: bool,
        /// Report what would change, without changing it
        #[arg(long)]
        dry_run: bool,
    },
    /// List decks with a new/learning/review/suspended breakdown, or manage them
    Decks {
        #[command(subcommand)]
        cmd: Option<DecksCmd>,
    },
    /// List notetypes (models), show one's fields, or change its fields/templates
    Models {
        /// Show field names of this notetype
        name: Option<String>,
        #[command(subcommand)]
        cmd: Option<ModelsCmd>,
    },
    /// Run as an MCP server over stdio (for `claude mcp add anki -- anki-cli mcp`)
    Mcp,
}

#[derive(Subcommand)]
enum DecksCmd {
    /// List decks with card counts (same as bare `decks`)
    Ls,
    /// Delete a deck and its child decks
    Rm {
        name: String,
        /// Delete the deck's notes along with it
        #[arg(long, conflicts_with = "keep_notes")]
        with_notes: bool,
        /// Move the deck's cards elsewhere first, so no note is lost
        #[arg(long)]
        keep_notes: bool,
        /// Where --keep-notes moves the cards
        #[arg(long, default_value = "Default")]
        to: String,
        /// Report what would be removed, without removing it
        #[arg(long)]
        dry_run: bool,
    },
    /// Rename a deck (its child decks follow)
    Mv { from: String, to: String },
}

#[derive(Subcommand)]
enum ModelsCmd {
    /// Show a notetype's fields and templates
    Show { name: String },
    /// Add a field. Schema change: a full `push` is needed afterwards
    AddField {
        model: String,
        field: String,
        /// 0-based position; appended at the end by default
        #[arg(long)]
        pos: Option<usize>,
    },
    /// Remove a field, losing its values. Schema change: a full `push` is needed
    RmField {
        model: String,
        field: String,
        /// Report how much data would be lost, without removing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Move a field to another position, keeping its values
    MvField {
        model: String,
        field: String,
        /// New 0-based position
        #[arg(long)]
        pos: usize,
    },
    /// Replace a card template's front/back. Schema change: a full `push` is needed
    EditTemplate {
        model: String,
        /// Which card template, 1-based (Anki's "Card 1")
        #[arg(long, default_value_t = 1)]
        card: usize,
        /// File holding the new front template
        #[arg(long)]
        front: Option<PathBuf>,
        /// File holding the new back template
        #[arg(long)]
        back: Option<PathBuf>,
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

/// Print an operation report in whichever form the caller asked for.
fn print_report(json: bool, report: &OpReport) {
    if json {
        print_json(report);
    } else {
        report.print_text();
    }
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

fn print_card_rows(rows: &[cards::CardRow]) {
    if rows.is_empty() {
        println!("No cards found.");
        return;
    }
    for c in rows {
        println!(
            "{}  {}  {}/{}  due={} ivl={} reps={} lapses={}  nid={}",
            c.card_id, c.deck, c.queue, c.ctype, c.due, c.ivl, c.reps, c.lapses, c.note_id
        );
    }
}

fn print_deck_rows(rows: &[decks::DeckRow]) {
    for d in rows {
        println!(
            "{}  ({} cards: new {}, learning {}, review {}, suspended {}, buried {})",
            d.name,
            d.cards,
            d.counts.new,
            d.counts.learning,
            d.counts.review,
            d.counts.suspended,
            d.counts.buried
        );
    }
}

fn print_model_info(m: &models::ModelInfo) {
    println!("notetype:  {}", m.name);
    println!("fields:    {}", m.fields.join(", "));
    println!("templates: {}", m.templates.join(", "));
}

fn read_template(path: &PathBuf) -> Result<String> {
    std::fs::read_to_string(path)
        .map_err(|e| anyhow!("reading template file {}: {e}", path.display()))
}

async fn run(cli: &Cli) -> Result<ExitCode> {
    if let Command::Mcp = &cli.command {
        // Directory resolution happens lazily per tool call, so the server
        // starts fine in a not-yet-initialized directory.
        mcp::serve(cli.dir.clone()).await?;
        return Ok(ExitCode::SUCCESS);
    }
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
        Command::Init | Command::Mcp => unreachable!("handled above"),
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
                    "queues:     new {}, learning {}, review {}, suspended {}, buried {}",
                    report.queues.new,
                    report.queues.learning,
                    report.queues.review,
                    report.queues.suspended,
                    report.queues.buried
                );
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
        Command::SyncMedia => {
            let report = sync::sync_media(&dir, &mut cfg).await?;
            if cli.json {
                print_json(&report);
            } else {
                println!(
                    "Media sync complete: {} file(s) in local media folder.",
                    report.media_files
                );
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
        Command::Search {
            query,
            limit,
            sort,
            sort_field,
            reverse,
        } => {
            let mut col = col::open_collection(&dir)?;
            let sel = Selector {
                query: Some(query.clone()),
                sort: *sort,
                sort_field: sort_field.clone(),
                reverse: *reverse,
                limit: Some(*limit),
                ..Default::default()
            };
            let nids = select::resolve_notes(&mut col, &sel)?;
            let results = notes::notes_info(&mut col, &nids)?;
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
        Command::Cards { sel } => {
            let mut col = col::open_collection(&dir)?;
            let cids = select::resolve_cards(&mut col, &sel.into())?;
            let rows = cards::card_rows(&mut col, &cids)?;
            if cli.json {
                print_json(&rows);
            } else {
                print_card_rows(&rows);
            }
        }
        Command::Show { note_id } => {
            let mut col = col::open_collection(&dir)?;
            let info = notes::note_info(&mut col, NoteId(*note_id))?;
            if cli.json {
                print_json(&info);
            } else {
                print_note_full(&info);
            }
        }
        Command::Edit {
            note_id,
            query,
            ids,
            limit,
            sort,
            sort_field,
            reverse,
            fields,
            add_tags,
            remove_tags,
            dry_run,
        } => {
            let named = notes::parse_field_args(fields)?;
            let add = notes::parse_tags(add_tags);
            let remove = notes::parse_tags(remove_tags);
            if named.is_empty() && add.is_empty() && remove.is_empty() {
                bail!("nothing to change: pass --field, --add-tags or --remove-tags");
            }
            let mut col = col::open_collection(&dir)?;
            match note_id {
                // A single id keeps the old behaviour: show the edited note.
                Some(note_id) if query.is_none() && ids.is_empty() => {
                    let info = notes::edit_note(&mut col, *note_id, &named, &add, &remove)?;
                    if cli.json {
                        print_json(&info);
                    } else {
                        print_note_full(&info);
                    }
                }
                _ => {
                    let sel = Selector {
                        query: query.clone(),
                        ids: ids.clone(),
                        sort: *sort,
                        sort_field: sort_field.clone(),
                        reverse: *reverse,
                        limit: *limit,
                        ..Default::default()
                    };
                    let nids = select::resolve_notes(&mut col, &sel)?;
                    let mut report = OpReport::new("edit", *dry_run, nids.len()).with_notes(&nids);
                    if !*dry_run {
                        let out = notes::edit_notes(&mut col, &nids, &named, &add, &remove)?;
                        report = report
                            .changed(out.changed())
                            .with_details(serde_json::to_value(&out)?);
                    }
                    print_report(cli.json, &report);
                }
            }
        }
        Command::Rm {
            note_ids,
            query,
            limit,
            dry_run,
        } => {
            let mut col = col::open_collection(&dir)?;
            let sel = Selector {
                query: query.clone(),
                ids: note_ids.clone(),
                limit: *limit,
                ..Default::default()
            };
            let nids = select::resolve_notes(&mut col, &sel)?;
            let mut report = OpReport::new("rm", *dry_run, nids.len()).with_notes(&nids);
            if !*dry_run {
                let removed = notes::remove_notes(&mut col, &nids)?;
                report = report
                    .changed(nids.len())
                    .with_details(serde_json::json!({"removed_cards": removed}));
            }
            print_report(cli.json, &report);
        }
        Command::Suspend { sel, dry_run } => {
            let mut col = col::open_collection(&dir)?;
            let cids = select::resolve_cards(&mut col, &sel.into())?;
            let mut report = OpReport::new("suspend", *dry_run, cids.len()).with_cards(&cids);
            if !*dry_run {
                report = report.changed(cards::suspend(&mut col, &cids)?);
            }
            print_report(cli.json, &report);
        }
        Command::Unsuspend { sel, dry_run } => {
            let mut col = col::open_collection(&dir)?;
            let cids = select::resolve_cards(&mut col, &sel.into())?;
            let mut report = OpReport::new("unsuspend", *dry_run, cids.len()).with_cards(&cids);
            if *dry_run {
                let suspended = cards::suspended_among(&mut col, &cids)?;
                report = report
                    .with_details(serde_json::json!({"currently_suspended": suspended.len()}));
            } else {
                report = report.changed(cards::unsuspend(&mut col, &cids)?);
            }
            print_report(cli.json, &report);
        }
        Command::Forget {
            sel,
            restore_position,
            reset_counts,
            dry_run,
        } => {
            let mut col = col::open_collection(&dir)?;
            let cids = select::resolve_cards(&mut col, &sel.into())?;
            let mut report = OpReport::new("forget", *dry_run, cids.len()).with_cards(&cids);
            if !*dry_run {
                let changed =
                    cards::forget(&mut col, &cids, *restore_position, *reset_counts)?;
                report = report.changed(changed);
            }
            print_report(cli.json, &report);
        }
        Command::Reposition {
            sel,
            start,
            step,
            randomize,
            shift,
            dry_run,
        } => {
            let mut col = col::open_collection(&dir)?;
            let cids = select::resolve_cards(&mut col, &sel.into())?;
            let mut report = OpReport::new("reposition", *dry_run, cids.len()).with_cards(&cids);
            if !*dry_run {
                let changed =
                    cards::reposition(&mut col, &cids, *start, *step, *randomize, *shift)?;
                report = report.changed(changed);
            }
            print_report(cli.json, &report);
        }
        Command::Mv {
            sel,
            deck,
            no_create,
            dry_run,
        } => {
            let mut col = col::open_collection(&dir)?;
            let cids = select::resolve_cards(&mut col, &sel.into())?;
            let mut report = OpReport::new("mv", *dry_run, cids.len()).with_cards(&cids);
            if *dry_run {
                report = report.with_details(serde_json::json!({"deck": deck}));
            } else {
                let (changed, name) = cards::move_to_deck(&mut col, &cids, deck, !*no_create)?;
                report = report
                    .changed(changed)
                    .with_details(serde_json::json!({"deck": name}));
            }
            print_report(cli.json, &report);
        }
        Command::Decks { cmd } => match cmd {
            None | Some(DecksCmd::Ls) => {
                let mut col = col::open_collection(&dir)?;
                let rows = decks::list(&mut col)?;
                if cli.json {
                    print_json(&rows);
                } else {
                    print_deck_rows(&rows);
                }
            }
            Some(DecksCmd::Rm {
                name,
                with_notes,
                keep_notes,
                to,
                dry_run,
            }) => {
                if !*with_notes && !*keep_notes {
                    bail!(
                        "say what happens to the notes: --with-notes deletes the deck's notes \
                         too, --keep-notes moves their cards to another deck (--to, default \
                         'Default') and deletes only the deck"
                    );
                }
                let mut col = col::open_collection(&dir)?;
                let preview = decks::removal_preview(&mut col, name)?;
                let matched = preview.cards;
                let mut report = OpReport::new("decks rm", *dry_run, matched).with_details(
                    serde_json::json!({
                        "deck": name,
                        "cards": preview.cards,
                        "notes_deleted": if *with_notes { preview.notes_deleted } else { 0 },
                        "notes_moved_to": if *keep_notes { Some(to.clone()) } else { None },
                    }),
                );
                if !*dry_run {
                    let out = decks::remove(&mut col, name, *keep_notes, to)?;
                    report = report
                        .changed(out.cards_affected())
                        .with_details(serde_json::json!({
                            "deck": name,
                            "cards_moved": out.cards_moved,
                            "cards_removed": out.cards_removed,
                            "notes_deleted": if *with_notes { preview.notes_deleted } else { 0 },
                            "notes_moved_to": out.moved_to,
                        }));
                }
                print_report(cli.json, &report);
            }
            Some(DecksCmd::Mv { from, to }) => {
                let mut col = col::open_collection(&dir)?;
                let name = decks::rename(&mut col, from, to)?;
                let report = OpReport::new("decks mv", false, 1)
                    .changed(1)
                    .with_details(serde_json::json!({"from": from, "to": name}));
                print_report(cli.json, &report);
            }
        },
        Command::Models { name, cmd } => {
            let mut col = col::open_collection(&dir)?;
            match (name, cmd) {
                (_, Some(ModelsCmd::Show { name })) | (Some(name), None) => {
                    let nt = models::get(&mut col, name)?;
                    let info = models::info(&nt);
                    if cli.json {
                        print_json(&info);
                    } else {
                        print_model_info(&info);
                    }
                }
                (_, Some(ModelsCmd::AddField { model, field, pos })) => {
                    let info = models::add_field(&mut col, model, field, *pos)?;
                    let report = OpReport::new("models add-field", false, 1)
                        .changed(1)
                        .needs_full_sync()
                        .with_details(serde_json::to_value(&info)?);
                    print_report(cli.json, &report);
                }
                (
                    _,
                    Some(ModelsCmd::RmField {
                        model,
                        field,
                        dry_run,
                    }),
                ) => {
                    let losing = models::notes_with_value(&mut col, model, field)?;
                    let mut report = OpReport::new("models rm-field", *dry_run, 1)
                        .needs_full_sync()
                        .with_details(serde_json::json!({
                            "model": model, "field": field, "notes_losing_data": losing,
                        }));
                    if !*dry_run {
                        let info = models::remove_field(&mut col, model, field)?;
                        report = report.changed(1).with_details(serde_json::json!({
                            "model": info.name,
                            "fields": info.fields,
                            "notes_losing_data": losing,
                        }));
                    }
                    print_report(cli.json, &report);
                }
                (_, Some(ModelsCmd::MvField { model, field, pos })) => {
                    let info = models::move_field(&mut col, model, field, *pos)?;
                    let report = OpReport::new("models mv-field", false, 1)
                        .changed(1)
                        .needs_full_sync()
                        .with_details(serde_json::to_value(&info)?);
                    print_report(cli.json, &report);
                }
                (
                    _,
                    Some(ModelsCmd::EditTemplate {
                        model,
                        card,
                        front,
                        back,
                    }),
                ) => {
                    let front = front.as_ref().map(read_template).transpose()?;
                    let back = back.as_ref().map(read_template).transpose()?;
                    let info = models::edit_template(&mut col, model, *card, front, back)?;
                    let report = OpReport::new("models edit-template", false, 1)
                        .changed(1)
                        .needs_full_sync()
                        .with_details(serde_json::to_value(&info)?);
                    print_report(cli.json, &report);
                }
                (None, None) => {
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
