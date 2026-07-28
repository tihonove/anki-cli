//! Turning `<QUERY>` / `--ids` / `--nids` plus `--sort` / `--limit` into a
//! concrete list of card or note ids.
//!
//! Every bulk command shares this: taking only ids would mean thousands of
//! calls to work through a 10k-note deck, and taking only a query would make
//! "just these three" awkward. Sorting matters because the typical workflow is
//! "unsuspend the next 10 by Rank" — and rslib can only order by its own
//! browser columns, so ordering by an arbitrary notetype field is done here.

use std::collections::HashMap;

use anki::browser_table::Column;
use anki::card::CardId;
use anki::collection::Collection;
use anki::notes::NoteId;
use anki::search::{SearchNode, SortMode};
use anyhow::{anyhow, bail, Result};

/// Card/note orderings rslib can do in SQL.
// `SortField` is Anki's own name for the notetype's sort column; renaming it to
// please the lint would just make it harder to match up with the browser.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Sort {
    /// Scheduled due date (new cards by queue position)
    Due,
    /// Position of a new card in the new-card queue
    Position,
    /// Note creation time
    Added,
    /// Note modification time
    Modified,
    /// Review interval
    Interval,
    Lapses,
    Reps,
    Ease,
    /// The notetype's sort field
    SortField,
    Deck,
    Tags,
}

impl std::str::FromStr for Sort {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        <Self as clap::ValueEnum>::from_str(s, true).map_err(|e| anyhow!("bad sort: {e}"))
    }
}

impl Sort {
    /// The accepted `--sort` values, for help text and MCP schemas.
    pub fn names() -> Vec<String> {
        use clap::ValueEnum;
        Sort::value_variants()
            .iter()
            .filter_map(|v| v.to_possible_value().map(|p| p.get_name().to_string()))
            .collect()
    }

    fn column(self) -> Column {
        match self {
            Sort::Due => Column::Due,
            Sort::Position => Column::OriginalPosition,
            Sort::Added => Column::NoteCreation,
            Sort::Modified => Column::NoteMod,
            Sort::Interval => Column::Interval,
            Sort::Lapses => Column::Lapses,
            Sort::Reps => Column::Reps,
            Sort::Ease => Column::Ease,
            Sort::SortField => Column::SortField,
            Sort::Deck => Column::Deck,
            Sort::Tags => Column::Tags,
        }
    }
}

/// What a command was pointed at, before it is resolved against the collection.
#[derive(Debug, Default, Clone)]
pub struct Selector {
    /// Anki search syntax, e.g. `deck:"A Frequency Dictionary of Dutch"`
    pub query: Option<String>,
    /// Card ids (card-level commands) or note ids (note-level commands)
    pub ids: Vec<i64>,
    /// Note ids, on card-level commands: "every card of these notes"
    pub nids: Vec<i64>,
    pub sort: Option<Sort>,
    /// Order by the value of this notetype field, numerically when it looks numeric
    pub sort_field: Option<String>,
    pub reverse: bool,
    pub limit: Option<usize>,
}

impl Selector {
    fn sort_mode(&self) -> SortMode {
        match self.sort {
            // A --sort-field pass reorders everything afterwards anyway.
            Some(_) if self.sort_field.is_some() => SortMode::NoOrder,
            Some(sort) => SortMode::Builtin {
                column: sort.column(),
                reverse: self.reverse,
            },
            None => SortMode::NoOrder,
        }
    }

    // Wording stays free of flag/argument spelling: the same error reaches CLI
    // users (`--ids`) and MCP callers (`card_ids`), and naming one misdirects
    // the other.
    fn check_one_source(&self) -> Result<()> {
        let sources = [
            self.query.is_some(),
            !self.ids.is_empty(),
            !self.nids.is_empty(),
        ]
        .iter()
        .filter(|set| **set)
        .count();
        if sources == 0 {
            bail!("nothing selected: pass a search query, card ids or note ids");
        }
        if sources > 1 {
            bail!("pass either a search query or explicit ids, not both");
        }
        if self.query.as_deref().is_some_and(str::is_empty) {
            bail!("empty search query would match the whole collection; pass a real query");
        }
        Ok(())
    }
}

/// Resolve to card ids: a query, explicit card ids, or every card of `--nids`.
pub fn resolve_cards(col: &mut Collection, sel: &Selector) -> Result<Vec<CardId>> {
    sel.check_one_source()?;
    let mode = sel.sort_mode();
    let mut cids = if let Some(query) = &sel.query {
        col.search_cards(query.as_str(), mode)
            .map_err(|e| anyhow!("search '{query}': {e}"))?
    } else if !sel.nids.is_empty() {
        let nids = sel.nids.iter().map(|&id| NoteId(id));
        col.search_cards(SearchNode::from_note_ids(nids), mode)?
    } else {
        let found = col.search_cards(
            SearchNode::from_card_ids(sel.ids.iter().map(|&id| CardId(id))),
            mode,
        )?;
        report_missing(&sel.ids, found.iter().map(|c| c.0), "card")?;
        found
    };

    if let Some(field) = &sel.sort_field {
        let keys = card_field_keys(col, &cids, field)?;
        sort_by_keys(&mut cids, &keys, sel.reverse);
    }
    truncate(&mut cids, sel.limit);
    Ok(cids)
}

/// Resolve to note ids: a query, or explicit note ids (`--ids` and `--nids`
/// mean the same thing here).
pub fn resolve_notes(col: &mut Collection, sel: &Selector) -> Result<Vec<NoteId>> {
    sel.check_one_source()?;
    let mode = sel.sort_mode();
    let mut nids = if let Some(query) = &sel.query {
        col.search_notes(query.as_str(), mode)
            .map_err(|e| anyhow!("search '{query}': {e}"))?
    } else {
        let ids: Vec<i64> = sel.ids.iter().chain(sel.nids.iter()).copied().collect();
        let found = col.search_notes(SearchNode::from_note_ids(ids.iter().map(|&id| NoteId(id))), mode)?;
        report_missing(&ids, found.iter().map(|n| n.0), "note")?;
        found
    };

    if let Some(field) = &sel.sort_field {
        let keys = note_field_keys(col, &nids, field)?;
        sort_by_keys(&mut nids, &keys, sel.reverse);
    }
    truncate(&mut nids, sel.limit);
    Ok(nids)
}

fn truncate<T>(items: &mut Vec<T>, limit: Option<usize>) {
    if let Some(limit) = limit {
        items.truncate(limit);
    }
}

/// Explicit ids that matched nothing are a mistake worth reporting, not a
/// silently smaller batch.
fn report_missing(asked: &[i64], found: impl Iterator<Item = i64>, kind: &str) -> Result<()> {
    let found: std::collections::HashSet<i64> = found.collect();
    let missing: Vec<String> = asked
        .iter()
        .filter(|id| !found.contains(id))
        .map(|id| id.to_string())
        .collect();
    if !missing.is_empty() {
        bail!("no {kind} with id {}", missing.join(", "));
    }
    Ok(())
}

/// A field value reduced to something comparable: numeric when every value
/// present looks like a number, so Rank 2 sorts before Rank 10.
#[derive(Debug, PartialEq)]
enum SortKey {
    Number(f64),
    Text(String),
    /// The note has no such field, or it is empty — sorted last either way.
    Missing,
}

impl SortKey {
    fn rank(&self) -> u8 {
        match self {
            SortKey::Number(_) | SortKey::Text(_) => 0,
            SortKey::Missing => 1,
        }
    }
}

fn sort_by_keys<T: Copy + Ord + std::hash::Hash + Eq>(
    items: &mut [T],
    keys: &HashMap<T, SortKey>,
    reverse: bool,
) {
    items.sort_by(|a, b| {
        let ka = keys.get(a).unwrap_or(&SortKey::Missing);
        let kb = keys.get(b).unwrap_or(&SortKey::Missing);
        let ord = ka.rank().cmp(&kb.rank()).then_with(|| match (ka, kb) {
            (SortKey::Number(x), SortKey::Number(y)) => {
                x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
            }
            (SortKey::Text(x), SortKey::Text(y)) => x.cmp(y),
            _ => std::cmp::Ordering::Equal,
        });
        // Ties keep a stable, reproducible order rather than SQLite's whim.
        let ord = ord.then_with(|| a.cmp(b));
        if reverse {
            ord.reverse()
        } else {
            ord
        }
    });
}

fn card_field_keys(
    col: &mut Collection,
    cids: &[CardId],
    field: &str,
) -> Result<HashMap<CardId, SortKey>> {
    let mut note_of_card = Vec::with_capacity(cids.len());
    for cid in cids {
        let card = col
            .storage
            .get_card(*cid)?
            .ok_or_else(|| anyhow!("no card with id {}", cid.0))?;
        note_of_card.push((*cid, card.note_id()));
    }
    let nids: Vec<NoteId> = note_of_card.iter().map(|(_, nid)| *nid).collect();
    let by_note = note_field_keys(col, &nids, field)?;
    Ok(note_of_card
        .into_iter()
        .map(|(cid, nid)| {
            let key = match by_note.get(&nid) {
                Some(SortKey::Number(n)) => SortKey::Number(*n),
                Some(SortKey::Text(t)) => SortKey::Text(t.clone()),
                _ => SortKey::Missing,
            };
            (cid, key)
        })
        .collect())
}

fn note_field_keys(
    col: &mut Collection,
    nids: &[NoteId],
    field: &str,
) -> Result<HashMap<NoteId, SortKey>> {
    let mut raw: Vec<(NoteId, Option<String>)> = Vec::with_capacity(nids.len());
    let mut seen = std::collections::HashSet::with_capacity(nids.len());
    let mut field_idx: HashMap<anki::notetype::NotetypeId, Option<usize>> = HashMap::new();
    for nid in nids {
        if !seen.insert(*nid) {
            continue;
        }
        let Some(note) = col.storage.get_note(*nid)? else {
            bail!("no note with id {}", nid.0);
        };
        let idx = match field_idx.get(&note.notetype_id) {
            Some(idx) => *idx,
            None => {
                let nt = col
                    .get_notetype(note.notetype_id)?
                    .ok_or_else(|| anyhow!("notetype of note {} missing", nid.0))?;
                let idx = nt
                    .fields
                    .iter()
                    .position(|f| f.name.eq_ignore_ascii_case(field));
                field_idx.insert(note.notetype_id, idx);
                idx
            }
        };
        let value = idx.and_then(|idx| note.fields().get(idx).cloned());
        raw.push((*nid, value));
    }
    if raw.iter().all(|(_, v)| v.is_none()) {
        let known: Vec<String> = field_idx.keys().filter_map(|ntid| {
            col.get_notetype(*ntid)
                .ok()
                .flatten()
                .map(|nt| format!("{}: {}", nt.name, nt.fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ")))
        }).collect();
        bail!(
            "no notetype among the selected notes has a field '{field}' ({})",
            known.join("; ")
        );
    }

    let cleaned: Vec<(NoteId, Option<String>)> = raw
        .into_iter()
        .map(|(nid, value)| {
            let value = value
                .map(|v| anki::text::strip_html(&v).trim().to_string())
                .filter(|v| !v.is_empty());
            (nid, value)
        })
        .collect();
    let all_numeric = cleaned
        .iter()
        .filter_map(|(_, v)| v.as_deref())
        .all(|v| v.parse::<f64>().is_ok());

    Ok(cleaned
        .into_iter()
        .map(|(nid, value)| {
            let key = match value {
                None => SortKey::Missing,
                Some(v) if all_numeric => SortKey::Number(v.parse().unwrap_or(f64::MAX)),
                Some(v) => SortKey::Text(v.to_lowercase()),
            };
            (nid, key)
        })
        .collect())
}
