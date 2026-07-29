//! Notetype surgery: adding, removing and repositioning fields, and editing
//! card templates.
//!
//! All of these are *schema changes*. rslib flags the collection accordingly
//! and a normal two-way sync will then report a conflict — the only way up is
//! a full `push`. Callers surface that in their output.
//!
//! The field `ord` values are the migration instruction: an existing field
//! keeps its `ord` so its values follow it, and a new field carries `ord:
//! None` so every note gets an empty value there. Nothing here renumbers them
//! by hand.

use std::sync::Arc;

use anki::collection::Collection;
use anki::notetype::{NoteField, Notetype};
use anki::search::{SearchNode, SortMode};
use anyhow::{anyhow, bail, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub fields: Vec<String>,
    pub templates: Vec<String>,
}

pub fn get(col: &mut Collection, name: &str) -> Result<Arc<Notetype>> {
    col.get_notetype_by_name(name)?.ok_or_else(|| {
        let names = col
            .storage
            .get_all_notetype_names()
            .map(|v| v.into_iter().map(|(_, n)| n).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        anyhow!("no notetype named '{name}' (available: {names})")
    })
}

pub fn info(nt: &Notetype) -> ModelInfo {
    ModelInfo {
        name: nt.name.clone(),
        fields: nt.fields.iter().map(|f| f.name.clone()).collect(),
        templates: nt.templates.iter().map(|t| t.name.clone()).collect(),
    }
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
                nt.fields
                    .iter()
                    .map(|f| f.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

/// Add a field, optionally at a given 0-based position. Existing notes keep
/// their values; the new field starts empty on all of them.
pub fn add_field(
    col: &mut Collection,
    model: &str,
    field: &str,
    pos: Option<usize>,
) -> Result<ModelInfo> {
    let nt = get(col, model)?;
    let mut nt = (*nt).clone();
    if nt.fields.iter().any(|f| f.name.eq_ignore_ascii_case(field)) {
        bail!("notetype '{}' already has a field '{field}'", nt.name);
    }
    let pos = match pos {
        Some(pos) if pos > nt.fields.len() => bail!(
            "--pos {pos} is past the end: notetype '{}' has {} field(s)",
            nt.name,
            nt.fields.len()
        ),
        Some(pos) => pos,
        None => nt.fields.len(),
    };
    nt.fields.insert(pos, NoteField::new(field));
    col.update_notetype(&mut nt, false)
        .map_err(|e| anyhow!("updating notetype '{model}': {e}"))?;
    Ok(info(get(col, model)?.as_ref()))
}

/// How many notes hold a non-empty value in this field — i.e. how much data
/// removing it would throw away.
pub fn notes_with_value(col: &mut Collection, model: &str, field: &str) -> Result<usize> {
    let nt = get(col, model)?;
    let idx = field_index(&nt, field)?;
    let nids = col.search_notes(SearchNode::from_notetype_name(model), SortMode::NoOrder)?;
    let mut count = 0;
    for nid in nids {
        let Some(note) = col.storage.get_note(nid)? else {
            continue;
        };
        if note.fields().get(idx).is_some_and(|v| !v.trim().is_empty()) {
            count += 1;
        }
    }
    Ok(count)
}

/// Remove a field. Its values are lost on every note, and references to it are
/// stripped from the card templates.
pub fn remove_field(col: &mut Collection, model: &str, field: &str) -> Result<ModelInfo> {
    let nt = get(col, model)?;
    let mut nt = (*nt).clone();
    let idx = field_index(&nt, field)?;
    if nt.fields.len() == 1 {
        bail!("notetype '{}' would be left with no fields", nt.name);
    }
    nt.fields.remove(idx);
    col.update_notetype(&mut nt, false)
        .map_err(|e| anyhow!("updating notetype '{model}': {e}"))?;
    Ok(info(get(col, model)?.as_ref()))
}

/// Move a field to another 0-based position, keeping its values (the `ord` it
/// already carries is what makes that work).
pub fn move_field(col: &mut Collection, model: &str, field: &str, pos: usize) -> Result<ModelInfo> {
    let nt = get(col, model)?;
    let mut nt = (*nt).clone();
    let idx = field_index(&nt, field)?;
    if pos >= nt.fields.len() {
        bail!(
            "--pos {pos} is past the end: notetype '{}' has {} field(s)",
            nt.name,
            nt.fields.len()
        );
    }
    let f = nt.fields.remove(idx);
    nt.fields.insert(pos, f);
    col.update_notetype(&mut nt, false)
        .map_err(|e| anyhow!("updating notetype '{model}': {e}"))?;
    Ok(info(get(col, model)?.as_ref()))
}

/// Replace a card template's front and/or back format. `card` is 1-based, to
/// match Anki's "Card 1".
pub fn edit_template(
    col: &mut Collection,
    model: &str,
    card: usize,
    front: Option<String>,
    back: Option<String>,
) -> Result<ModelInfo> {
    if front.is_none() && back.is_none() {
        bail!("nothing to change: pass --front and/or --back");
    }
    let nt = get(col, model)?;
    let mut nt = (*nt).clone();
    if card == 0 || card > nt.templates.len() {
        bail!(
            "notetype '{}' has {} card template(s); --card is 1-based",
            nt.name,
            nt.templates.len()
        );
    }
    let template = &mut nt.templates[card - 1];
    if let Some(front) = front {
        template.config.q_format = front;
    }
    if let Some(back) = back {
        template.config.a_format = back;
    }
    col.update_notetype(&mut nt, false)
        .map_err(|e| anyhow!("updating notetype '{model}': {e}"))?;
    Ok(info(get(col, model)?.as_ref()))
}
