//! The result shape every bulk/destructive command reports: how much was
//! matched, how much actually changed, and — under `--dry-run` — what *would*
//! have changed. Uniform on purpose: an agent driving `--json` should not have
//! to learn a different envelope per command.

use anki::card::CardId;
use anki::notes::NoteId;
use serde::Serialize;
use serde_json::Value;

/// How many ids to name in the human-readable output before trimming.
const SAMPLE: usize = 10;

#[derive(Debug, Serialize)]
pub struct OpReport {
    pub op: String,
    pub dry_run: bool,
    /// Cards/notes the selector picked out.
    pub matched: usize,
    /// Of those, how many the operation actually altered (0 for a dry run).
    pub changed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cards: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vec<i64>>,
    /// Set by schema changes: a normal `sync` will conflict, `push` is required.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub full_sync_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl OpReport {
    pub fn new(op: &str, dry_run: bool, matched: usize) -> Self {
        OpReport {
            op: op.to_string(),
            dry_run,
            matched,
            changed: 0,
            cards: None,
            notes: None,
            full_sync_required: false,
            details: None,
        }
    }

    pub fn changed(mut self, changed: usize) -> Self {
        self.changed = changed;
        self
    }

    pub fn with_cards(mut self, cids: &[CardId]) -> Self {
        self.cards = Some(cids.iter().map(|c| c.0).collect());
        self
    }

    pub fn with_notes(mut self, nids: &[NoteId]) -> Self {
        self.notes = Some(nids.iter().map(|n| n.0).collect());
        self
    }

    pub fn needs_full_sync(mut self) -> Self {
        self.full_sync_required = true;
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// One-or-two-line summary for humans; `--json` prints the struct instead.
    pub fn print_text(&self) {
        let subject = match (&self.cards, &self.notes) {
            (Some(_), _) => "card",
            (None, Some(_)) => "note",
            _ => "item",
        };
        if self.dry_run {
            println!(
                "[dry run] {}: {} {subject}(s) would be affected.",
                self.op, self.matched
            );
        } else {
            println!(
                "{}: {} {subject}(s) changed ({} matched).",
                self.op, self.changed, self.matched
            );
        }
        if let Some(ids) = self.cards.as_ref().or(self.notes.as_ref()) {
            if !ids.is_empty() {
                let shown: Vec<String> = ids.iter().take(SAMPLE).map(|i| i.to_string()).collect();
                let more = ids.len().saturating_sub(shown.len());
                let tail = if more > 0 {
                    format!(" … (+{more} more)")
                } else {
                    String::new()
                };
                println!("  {}{tail}", shown.join(", "));
            }
        }
        if let Some(details) = &self.details {
            if let Some(obj) = details.as_object() {
                for (key, value) in obj {
                    println!("  {key}: {value}");
                }
            }
        }
        if self.full_sync_required {
            println!(
                "  schema change: a normal `anki-cli sync` will report a conflict — \
                 run `anki-cli push` to upload the new schema."
            );
        }
    }
}
