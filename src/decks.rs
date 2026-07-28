//! Deck listing with a queue breakdown, plus deletion and renaming.
//!
//! The breakdown exists to answer "how much is still in the pipeline" without
//! opening sqlite. rslib's `deck_tree` can't serve it: it has no suspended
//! count and it applies the deck's daily limits, whereas the question here is
//! about the collection's contents, not today's study plan.

use anki::collection::Collection;
use anki::decks::DeckId;
use anki::search::{SearchBuilder, SearchNode, SortMode};
use anyhow::{anyhow, bail, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DeckRow {
    pub id: i64,
    pub name: String,
    pub cards: usize,
    #[serde(flatten)]
    pub counts: crate::cards::QueueCounts,
}

/// An escaped `deck:"…"` search term for a deck name that may contain spaces,
/// `*`, `_` or quotes.
pub fn deck_term(name: &str) -> String {
    SearchBuilder::from(SearchNode::from_deck_name(name)).write()
}

fn count(col: &mut Collection, query: &str) -> Result<usize> {
    Ok(col.search_cards(query, SortMode::NoOrder)?.len())
}

/// Counts per queue for one deck, including its child decks (as Anki's `deck:`
/// search does).
pub fn deck_counts(col: &mut Collection, name: &str) -> Result<DeckRow> {
    let id = col.get_deck_id(name)?.unwrap_or(DeckId(0));
    let deck = deck_term(name);
    Ok(DeckRow {
        id: id.0,
        name: name.to_string(),
        cards: count(col, &deck)?,
        counts: crate::cards::queue_counts(col, &deck)?,
    })
}

pub fn list(col: &mut Collection) -> Result<Vec<DeckRow>> {
    let names = col.get_all_deck_names(false)?;
    let mut out = Vec::with_capacity(names.len());
    for (id, name) in names {
        let mut row = deck_counts(col, &name)?;
        row.id = id.0;
        out.push(row);
    }
    Ok(out)
}

fn deck_id(col: &mut Collection, name: &str) -> Result<DeckId> {
    col.get_deck_id(name)?
        .ok_or_else(|| anyhow!("no deck named '{name}'"))
}

#[derive(Debug, Serialize)]
pub struct RemovalPreview {
    pub cards: usize,
    /// Notes that would be deleted outright (all of their cards live here).
    pub notes_deleted: usize,
    /// Notes that keep at least one card in another deck.
    pub notes_kept: usize,
}

/// What deleting this deck would cost, before deciding to do it.
pub fn removal_preview(col: &mut Collection, name: &str) -> Result<RemovalPreview> {
    let deck = deck_term(name);
    let cards = count(col, &deck)?;
    let inside = col.search_notes(deck.as_str(), SortMode::NoOrder)?;
    let outside: std::collections::HashSet<_> = col
        .search_notes(format!("-{deck}").as_str(), SortMode::NoOrder)?
        .into_iter()
        .collect();
    let notes_kept = inside.iter().filter(|nid| outside.contains(nid)).count();
    Ok(RemovalPreview {
        cards,
        notes_deleted: inside.len() - notes_kept,
        notes_kept,
    })
}

#[derive(Debug, Serialize)]
pub struct Removal {
    /// Cards moved out of the deck before it went (`--keep-notes`).
    pub cards_moved: usize,
    /// Cards deleted along with the deck (`--with-notes`).
    pub cards_removed: usize,
    pub moved_to: Option<String>,
}

impl Removal {
    /// Cards this touched, either way.
    pub fn cards_affected(&self) -> usize {
        self.cards_moved + self.cards_removed
    }
}

/// Delete a deck and its children. `keep_notes` first moves every card to
/// `move_to`, because rslib's deck removal deletes the cards it finds — and
/// with them any note left without cards.
pub fn remove(col: &mut Collection, name: &str, keep_notes: bool, move_to: &str) -> Result<Removal> {
    let did = deck_id(col, name)?;
    let mut cards_moved = 0;
    let mut moved_to = None;
    if keep_notes {
        let cids = col.search_cards(deck_term(name).as_str(), SortMode::NoOrder)?;
        if !cids.is_empty() {
            let target = col.get_or_create_normal_deck(move_to)?;
            if target.id == did {
                bail!("--to '{move_to}' is the deck being removed");
            }
            let out = col
                .set_deck(&cids, target.id)
                .map_err(|e| anyhow!("moving cards to '{move_to}': {e}"))?;
            cards_moved = out.output;
            moved_to = Some(target.human_name());
        }
    }
    let out = col
        .remove_decks_and_child_decks(&[did])
        .map_err(|e| anyhow!("removing deck '{name}': {e}"))?;
    Ok(Removal {
        cards_moved,
        cards_removed: out.output,
        moved_to,
    })
}

/// Rename a deck. Child decks follow automatically; a clashing name is
/// uniquified by rslib rather than rejected, so the real name is read back.
pub fn rename(col: &mut Collection, from: &str, to: &str) -> Result<String> {
    let did = deck_id(col, from)?;
    col.rename_deck(did, to)
        .map_err(|e| anyhow!("renaming deck '{from}': {e}"))?;
    Ok(col
        .get_deck(did)?
        .map(|d| d.human_name())
        .unwrap_or_else(|| to.to_string()))
}
