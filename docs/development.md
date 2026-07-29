# Development

Building `anki-cli` from source, cutting releases, and how the collection is stored on disk.
For everyday use, see the [README](../README.md).

## Build

`anki-cli` depends on Anki's `rslib` as a pinned **git dependency**, so cargo fetches the
source (and its i18n submodules) itself — nothing to clone or vendor. You only need `protoc`
(used by `anki_proto`'s build script):

```bash
# protoc: apt install protobuf-compiler, or a binary from
# github.com/protocolbuffers/protobuf/releases; see .cargo/config.toml, which sets PROTOC.
cargo build --release          # binary at target/release/anki-cli
cargo test                     # local integration tests (no network)
```

The easiest path is the dev container in [`.devcontainer/`](../.devcontainer/README.md): it
ships a fresh Rust toolchain and `protoc` preconfigured, so `cargo build` just works.

## Releasing

CI builds the binaries and attaches them to a GitHub release when a `vX.Y.Z` tag is pushed.
To cut one:

```bash
scripts/release.sh patch       # e.g. 0.1.0 -> 0.1.1: bump Cargo.toml, tag, push -> CI publishes
scripts/release.sh minor       # 0.1.0 -> 0.2.0
scripts/release.sh major       # 0.1.0 -> 1.0.0
DRY_RUN=1 scripts/release.sh patch   # everything except the push
```

## What's inside

- `.anki/` holds `collection.anki2` (a regular SQLite DB with Anki's schema — openable in
  desktop Anki), `config.json`, `collection.media/`, and a `.gitignore` with `*` so none of it
  accidentally lands in git.
- The session key (hkey) is stored in `.anki/config.json` in the clear (mode 0600) — same as
  desktop Anki. The password is not stored. `logout` erases the key.
- AnkiWeb redirects to a shard (e.g. `sync11.ankiweb.net`) — the CLI picks that up and
  remembers the endpoint.
- Media files (images/audio referenced by notes) sync with `anki-cli sync-media`, kept in
  `.anki/collection.media`. It's a separate step from collection `sync`: uploads local
  additions and downloads server-side additions/deletions, merged file-by-file (never
  conflicts). See `sync::sync_media`, which drives anki's `MediaManager::sync_media`.
- Answering cards (the review loop) isn't exposed in the CLI — the assumption is that you
  study in regular Anki. Queue management around it is: see `cards.rs`.
- License: `rslib` is AGPL-3.0, so this tool is AGPL-3.0 too.

## Source layout

| file | what lives there |
|---|---|
| `main.rs` | the clap command tree and all human-readable printing |
| `select.rs` | `<QUERY>` / `--ids` / `--nids` + `--sort` / `--sort-field` / `--limit` → concrete ids |
| `ops.rs` | `OpReport`, the single result envelope every bulk command returns |
| `cards.rs` | card listing, suspend/unsuspend/forget/reposition, moving between decks |
| `notes.rs` | note CRUD, and the bulk edit path |
| `decks.rs` | deck listing with queue counts, removal, renaming |
| `models.rs` | notetype field/template surgery |
| `sync.rs` | login, sync/pull/push, media sync, status |
| `mcp.rs` | the MCP server: one tool per CLI operation |

Three rslib constraints shaped the above, and are worth knowing before extending it:

- **`anki::card::Card`'s fields are `pub(crate)`.** Queue, due and interval are read through
  the public `From<Card> for anki_proto::cards::Card` conversion (see `cards.rs`).
- **`Collection::transact` is `pub(crate)`,** so a bulk operation can't be wrapped in one
  transaction of our own. Bulk note edits therefore go through rslib's own single-transaction
  entry points: `add_tags_to_notes` / `remove_tags_from_notes`, and `NotesService::update_notes`.
- **`Notetype::add_field` is `pub(crate)`.** Fields are pushed onto `nt.fields` directly; the
  `ord` each existing field carries is what tells rslib where its values used to live, so
  `ord`s are never renumbered by hand (`notetype/schemachange.rs` does the migration).

Deck removal deletes the cards it finds, and with them any note left without cards — which is
why `decks rm --keep-notes` moves the cards out first rather than passing a flag.
