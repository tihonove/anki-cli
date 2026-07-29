//! Card-level operations: the scheduling side of the collection.
//!
//! `search` answers questions about notes; this module answers "what is
//! actually in the queue right now" and changes it — suspend/unsuspend,
//! forget, reposition, move between decks.
//!
//! Note on reading cards: every field of `anki::card::Card` is `pub(crate)`,
//! so queue/due/ivl are unreachable from here. The public route is the
//! `From<Card> for anki_proto::cards::Card` conversion, which hands over the
//! same values as plain integers.

use std::collections::HashMap;

use anki::card::{CardId, CardQueue, CardType};
use anki::collection::Collection;
use anki::decks::DeckId;
use anki::scheduler::new::NewCardDueOrder;
use anki::search::SortMode;
use anki_proto::scheduler::bury_or_suspend_cards_request::Mode as BuryOrSuspendMode;
use anyhow::{anyhow, bail, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CardRow {
    pub card_id: i64,
    pub note_id: i64,
    pub deck: String,
    /// Which card of the note this is (0-based, as in Anki's "card 1" = 0)
    pub template_idx: u32,
    pub queue: String,
    pub ctype: String,
    /// New cards: queue position. Review cards: days since collection creation.
    pub due: i32,
    /// Review interval in days
    pub ivl: u32,
    pub reps: u32,
    pub lapses: u32,
    /// Ease factor in permille (2500 = 250%)
    pub ease: u32,
    /// Original new-queue position, kept while the card is out of the new queue
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<u32>,
}

fn queue_name(queue: i32) -> String {
    match CardQueue::try_from(queue as i8) {
        Ok(CardQueue::New) => "new",
        Ok(CardQueue::Learn) => "learn",
        Ok(CardQueue::Review) => "review",
        Ok(CardQueue::DayLearn) => "day_learn",
        Ok(CardQueue::PreviewRepeat) => "preview_repeat",
        Ok(CardQueue::Suspended) => "suspended",
        Ok(CardQueue::SchedBuried) => "sched_buried",
        Ok(CardQueue::UserBuried) => "user_buried",
        Err(_) => return format!("queue#{queue}"),
    }
    .to_string()
}

fn type_name(ctype: u32) -> String {
    match CardType::try_from(ctype as u8) {
        Ok(CardType::New) => "new",
        Ok(CardType::Learn) => "learn",
        Ok(CardType::Review) => "review",
        Ok(CardType::Relearn) => "relearn",
        Err(_) => return format!("type#{ctype}"),
    }
    .to_string()
}

/// Fetch the given cards as printable rows, in the order asked for.
pub fn card_rows(col: &mut Collection, cids: &[CardId]) -> Result<Vec<CardRow>> {
    let mut deck_names: HashMap<DeckId, String> = HashMap::new();
    let mut rows = Vec::with_capacity(cids.len());
    for cid in cids {
        let Some(card) = col.storage.get_card(*cid)? else {
            bail!("no card with id {}", cid.0);
        };
        let deck_id = card.deck_id();
        let deck = match deck_names.get(&deck_id) {
            Some(name) => name.clone(),
            None => {
                let name = col
                    .get_deck(deck_id)?
                    .map(|d| d.human_name())
                    .unwrap_or_else(|| format!("deck#{}", deck_id.0));
                deck_names.insert(deck_id, name.clone());
                name
            }
        };
        let p: anki_proto::cards::Card = card.into();
        rows.push(CardRow {
            card_id: p.id,
            note_id: p.note_id,
            deck,
            template_idx: p.template_idx,
            queue: queue_name(p.queue),
            ctype: type_name(p.ctype),
            due: p.due,
            ivl: p.interval,
            reps: p.reps,
            lapses: p.lapses,
            ease: p.ease_factor,
            position: p.original_position,
        });
    }
    Ok(rows)
}

/// A breakdown of where cards currently sit. The five buckets are mutually
/// exclusive and add up to the total, so "how much is still in the pipeline"
/// is answerable without opening sqlite.
#[derive(Debug, Serialize)]
pub struct QueueCounts {
    pub new: usize,
    pub learning: usize,
    pub review: usize,
    pub suspended: usize,
    pub buried: usize,
}

/// Count by queue within `scope` (an Anki search fragment; empty = the whole
/// collection).
pub fn queue_counts(col: &mut Collection, scope: &str) -> Result<QueueCounts> {
    let count = |col: &mut Collection, extra: &str| -> Result<usize> {
        let query = format!("{scope} {extra}");
        Ok(col.search_cards(query.trim(), SortMode::NoOrder)?.len())
    };
    Ok(QueueCounts {
        new: count(col, "is:new -is:suspended -is:buried")?,
        learning: count(col, "is:learn -is:suspended -is:buried")?,
        review: count(col, "is:review -is:learn -is:suspended -is:buried")?,
        suspended: count(col, "is:suspended")?,
        buried: count(col, "is:buried")?,
    })
}

/// Card ids among `cids` that are currently suspended.
pub fn suspended_among(col: &mut Collection, cids: &[CardId]) -> Result<Vec<CardId>> {
    let mut out = Vec::new();
    for cid in cids {
        let Some(card) = col.storage.get_card(*cid)? else {
            bail!("no card with id {}", cid.0);
        };
        let p: anki_proto::cards::Card = card.into();
        if CardQueue::try_from(p.queue as i8) == Ok(CardQueue::Suspended) {
            out.push(*cid);
        }
    }
    Ok(out)
}

pub fn suspend(col: &mut Collection, cids: &[CardId]) -> Result<usize> {
    let out = col
        .bury_or_suspend_cards(cids, BuryOrSuspendMode::Suspend)
        .map_err(|e| anyhow!("suspending cards: {e}"))?;
    Ok(out.output)
}

/// Unsuspend, and only unsuspend: rslib's call also unburies, so buried cards
/// are filtered out first. That filter doubles as the changed-count, which the
/// underlying op does not report.
pub fn unsuspend(col: &mut Collection, cids: &[CardId]) -> Result<usize> {
    let suspended = suspended_among(col, cids)?;
    if suspended.is_empty() {
        return Ok(0);
    }
    col.unbury_or_unsuspend_cards(&suspended)
        .map_err(|e| anyhow!("unsuspending cards: {e}"))?;
    Ok(suspended.len())
}

pub fn forget(
    col: &mut Collection,
    cids: &[CardId],
    restore_position: bool,
    reset_counts: bool,
) -> Result<usize> {
    col.reschedule_cards_as_new(cids, true, restore_position, reset_counts, None)
        .map_err(|e| anyhow!("forgetting cards: {e}"))?;
    Ok(cids.len())
}

/// Renumber the new-card queue. `cids` is used in the order given, so pairing
/// this with `--sort-field Rank` is what puts a frequency deck in frequency
/// order. Only cards still in the new queue are affected.
pub fn reposition(
    col: &mut Collection,
    cids: &[CardId],
    start: u32,
    step: u32,
    randomize: bool,
    shift: bool,
) -> Result<usize> {
    let order = if randomize {
        NewCardDueOrder::Random
    } else {
        NewCardDueOrder::Preserve
    };
    let out = col
        .sort_cards(cids, start, step, order, shift)
        .map_err(|e| anyhow!("repositioning cards: {e}"))?;
    Ok(out.output)
}

/// Move cards to another deck. Returns how many moved and the deck's real name
/// (rslib may uniquify a freshly created one).
pub fn move_to_deck(
    col: &mut Collection,
    cids: &[CardId],
    deck: &str,
    create: bool,
) -> Result<(usize, String)> {
    let (deck_id, name) = if create {
        let d = col.get_or_create_normal_deck(deck)?;
        (d.id, d.human_name())
    } else {
        let id = col
            .get_deck_id(deck)?
            .ok_or_else(|| anyhow!("no deck named '{deck}' (drop --no-create to create it)"))?;
        let name = col
            .get_deck(id)?
            .map(|d| d.human_name())
            .unwrap_or_else(|| deck.to_string());
        (id, name)
    };
    let out = col
        .set_deck(cids, deck_id)
        .map_err(|e| anyhow!("moving cards to '{name}': {e}"))?;
    Ok((out.output, name))
}
