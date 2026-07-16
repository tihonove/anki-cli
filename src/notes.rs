use std::collections::HashMap;
use std::sync::Arc;

use anki::collection::Collection;
use anki::decks::DeckId;
use anki::notes::{Note, NoteId};
use anki::notetype::Notetype;
use anki::search::SortMode;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct NoteInfo {
    pub note_id: i64,
    pub model: String,
    pub fields: Vec<FieldValue>,
    pub tags: Vec<String>,
    pub cards: Vec<CardInfo>,
}

#[derive(Debug, Serialize)]
pub struct FieldValue {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Serialize)]
pub struct CardInfo {
    pub card_id: i64,
    pub deck: String,
}

/// Parse repeated `Name=Value` args.
pub fn parse_field_args(args: &[String]) -> Result<Vec<(String, String)>> {
    args.iter()
        .map(|arg| {
            arg.split_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.to_string()))
                .ok_or_else(|| anyhow!("field must be in Name=Value form, got: {arg}"))
        })
        .collect()
}

/// Split a tags argument on commas/whitespace.
pub fn parse_tags(tags: &str) -> Vec<String> {
    tags.split([',', ' '])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn field_index(nt: &Notetype, name: &str) -> Result<usize> {
    nt.fields
        .iter()
        .position(|f| f.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            anyhow!(
                "notetype '{}' has no field '{}' (fields: {})",
                nt.name,
                name,
                nt.fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ")
            )
        })
}

fn get_notetype(col: &mut Collection, name: &str) -> Result<Arc<Notetype>> {
    col.get_notetype_by_name(name)?.ok_or_else(|| {
        let names = col
            .storage
            .get_all_notetype_names()
            .map(|v| v.into_iter().map(|(_, n)| n).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        anyhow!("no notetype named '{name}' (available: {names})")
    })
}

pub fn add_note(
    col: &mut Collection,
    deck: &str,
    model: &str,
    positional: &[String],
    named_fields: &[(String, String)],
    tags: &[String],
) -> Result<NoteInfo> {
    let deck = col.get_or_create_normal_deck(deck)?;
    let nt = get_notetype(col, model)?;
    if positional.len() > nt.fields.len() {
        bail!(
            "too many field values: notetype '{}' has {} fields",
            nt.name,
            nt.fields.len()
        );
    }
    let mut note = nt.new_note();
    for (idx, value) in positional.iter().enumerate() {
        note.set_field(idx, value.clone()).map_err(|e| anyhow!("{e}"))?;
    }
    for (name, value) in named_fields {
        let idx = field_index(&nt, name)?;
        note.set_field(idx, value.clone()).map_err(|e| anyhow!("{e}"))?;
    }
    note.tags = tags.to_vec();
    col.add_note(&mut note, deck.id)
        .map_err(|e| anyhow!("adding note: {e}"))?;
    note_info(col, note.id)
}

pub fn edit_note(
    col: &mut Collection,
    note_id: i64,
    named_fields: &[(String, String)],
    add_tags: &[String],
    remove_tags: &[String],
) -> Result<NoteInfo> {
    let nid = NoteId(note_id);
    let mut note = col
        .storage
        .get_note(nid)?
        .with_context(|| format!("no note with id {note_id}"))?;
    let nt = col
        .get_notetype(note.notetype_id)?
        .context("notetype of note missing")?;
    for (name, value) in named_fields {
        let idx = field_index(&nt, name)?;
        note.set_field(idx, value.clone()).map_err(|e| anyhow!("{e}"))?;
    }
    for tag in add_tags {
        if !note.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
            note.tags.push(tag.clone());
        }
    }
    note.tags
        .retain(|t| !remove_tags.iter().any(|r| r.eq_ignore_ascii_case(t)));
    col.update_note(&mut note).map_err(|e| anyhow!("updating note: {e}"))?;
    note_info(col, nid)
}

pub fn remove_notes(col: &mut Collection, note_ids: &[i64]) -> Result<usize> {
    let nids: Vec<NoteId> = note_ids.iter().map(|&id| NoteId(id)).collect();
    for nid in &nids {
        if col.storage.get_note(*nid)?.is_none() {
            bail!("no note with id {}", nid.0);
        }
    }
    let out = col.remove_notes(&nids).map_err(|e| anyhow!("removing notes: {e}"))?;
    Ok(out.output)
}

pub fn search_notes(col: &mut Collection, query: &str, limit: usize) -> Result<Vec<NoteInfo>> {
    let nids = col
        .search_notes(query, SortMode::NoOrder)
        .map_err(|e| anyhow!("search '{query}': {e}"))?;
    nids.iter()
        .take(limit)
        .map(|&nid| note_info(col, nid))
        .collect()
}

pub fn note_info(col: &mut Collection, nid: NoteId) -> Result<NoteInfo> {
    let note = col
        .storage
        .get_note(nid)?
        .with_context(|| format!("no note with id {}", nid.0))?;
    let nt = col
        .get_notetype(note.notetype_id)?
        .context("notetype of note missing")?;
    let cards = cards_of_note(col, &note)?;
    let fields = nt
        .fields
        .iter()
        .zip(note.fields().iter())
        .map(|(f, v)| FieldValue {
            name: f.name.clone(),
            value: v.clone(),
        })
        .collect();
    Ok(NoteInfo {
        note_id: note.id.0,
        model: nt.name.clone(),
        fields,
        tags: note.tags.clone(),
        cards,
    })
}

fn cards_of_note(col: &mut Collection, note: &Note) -> Result<Vec<CardInfo>> {
    let cids = col.search_cards(format!("nid:{}", note.id.0).as_str(), SortMode::NoOrder)?;
    let mut deck_names: HashMap<DeckId, String> = HashMap::new();
    let mut infos = Vec::new();
    for cid in cids {
        let Some(card) = col.storage.get_card(cid)? else {
            continue;
        };
        let deck = match deck_names.get(&card.deck_id()) {
            Some(name) => name.clone(),
            None => {
                let name = col
                    .get_deck(card.deck_id())?
                    .map(|d| d.human_name())
                    .unwrap_or_else(|| format!("deck#{}", card.deck_id().0));
                deck_names.insert(card.deck_id(), name.clone());
                name
            }
        };
        infos.push(CardInfo {
            card_id: cid.0,
            deck,
        });
    }
    Ok(infos)
}
