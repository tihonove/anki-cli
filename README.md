# anki-cli

A git-like CLI for Anki: keep a local copy of your collection, edit it from the terminal
(or from an agent), and sync with AnkiWeb with explicit conflict resolution.

Built on top of the official `rslib` (Anki's Rust core, [ankitects/anki](https://github.com/ankitects/anki)) —
so the database, schema, and sync protocol are exactly the same as real Anki. A single
self-contained binary.

## Mental model (like git)

| git | anki-cli | what it does |
|---|---|---|
| `clone` / `reset --hard origin` | `pull` | full download of the collection from the server (overwrites local) |
| `push --force` | `push` | full upload of the local collection to the server (overwrites remote) |
| `pull` + `push` (merge) | `sync` | normal two-way sync; **exit code 2** on conflict |
| `status` | `status` | local changes + whether the server differs |

A conflict (after a schema change, a `push` from another device, etc.) can't be resolved by a
normal `sync` — the command prints the options and exits with code 2. Resolve it with `pull`
(take the server version) or `push` (take the local version).

`pull` refuses to overwrite unsynced local changes — add `--force` to accept the loss.

## Quick start

The collection lives in a project directory (git model): `init` creates `./.anki/`, and the
other commands look for it upward from the current directory. Each such directory has its own
account and its own session key, so multiple accounts = multiple directories.

```bash
cd ~/lang/deutsch
anki-cli init                                     # creates ./.anki (like git init)
anki-cli login -u you@example.com -p 'password'   # or env: ANKI_USERNAME / ANKI_PASSWORD
anki-cli pull                                     # fetch the collection from AnkiWeb

anki-cli add -d "Deutsch::A1" "der Hund" "dog" -t "noun a1"
anki-cli add -m "Basic (and reversed card)" --field Front="die Katze" --field Back="cat"
anki-cli add -m Cloze "Der {{c1::Hund}} bellt."

anki-cli sync                                     # push changes up (two-way merge)
```

## Commands

```
init                                          create ./.anki in the current directory
login -u <email> -p <pass> [--endpoint URL]   log in; session key saved in .anki/config.json (0600)
logout                                        forget the session key
status [--offline]                            notes/cards, local changes, server status
sync                                          two-way sync (exit 2 = conflict)
pull [--force]                                full download from the server
push                                          full upload to the server

add [-d DECK] [-m MODEL] [field values...] [--field Name=Value]... [-t "tags"]
search <query> [--limit N]                    Anki search syntax: deck:X tag:Y word
show <note_id>                                the full note
edit <note_id> [--field Name=Value]... [--add-tags "..."] [--remove-tags "..."]
rm <note_id>...                               delete notes (with their cards)
decks                                         list decks with card counts
models [name]                                 list notetypes / fields of a specific notetype
```

Global flags:

- `--json` — machine-readable output (for agents); errors go to stderr as JSON.
- `--dir PATH` — point at the data directory explicitly, skipping the `.anki/` search.
  Priority: `--dir` > `$ANKI_CLI_HOME` > nearest `.anki/` up the tree; if none, an error with
  a hint about `init`.

## Example: an agent's work loop

```bash
anki-cli pull
anki-cli --json search "deck:Spanish tag:verb"       # what's already there
anki-cli --json add -d Spanish "el perro" "dog" -t noun
anki-cli sync || {
  # exit 2: the collections diverged — resolve in favour of the local version
  anki-cli push
}
```

## MCP for Claude

The same binary can act as an MCP server (stdio) — nothing extra to install:

```bash
cd ~/lang/deutsch          # a directory with .anki (init already done)
claude mcp add anki -- anki-cli mcp
```

The server looks for `.anki/` upward from the working directory (Claude Code starts MCP
servers in the project directory), so each project automatically uses its own collection and
its own account.

Authenticate with the **`anki_login`** tool, or run `anki-cli login` once beforehand. To keep
the password out of the conversation, `anki_login` falls back to the server's
`ANKI_USERNAME` / `ANKI_PASSWORD` environment variables when its arguments are omitted —
either way only the session key is stored, never the password.

Tools: `anki_login`, `anki_logout`, `anki_status`, `anki_sync`, `anki_pull`, `anki_push`,
`anki_add_note`, `anki_add_notes` (bulk), `anki_search`, `anki_get_note`, `anki_edit_note`, `anki_delete_notes`,
`anki_list_decks`, `anki_list_models`. A sync conflict reaches the agent as
`result: "conflict"` with a hint, resolved by calling `anki_pull`/`anki_push`.

## Install

One-liner (Linux x86_64, macOS arm64):

```bash
curl -fsSL https://raw.githubusercontent.com/tihonove/anki-cli/main/install.sh | sh
```

It downloads the prebuilt binary into `~/.local/bin` (override with `ANKI_CLI_BIN`, or pin a
release with `ANKI_CLI_VERSION=vX.Y.Z`). Prefer to do it by hand? Grab the raw binary for your
platform from the [releases](https://github.com/tihonove/anki-cli/releases), `chmod +x`, and
put it on your `PATH`. Or build it yourself:

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

The easiest path is the dev container in [`.devcontainer/`](.devcontainer/README.md): it ships
a fresh Rust toolchain and `protoc` preconfigured, so `cargo build` just works.

## Releasing

CI builds the binaries and attaches them to a GitHub release when a `vX.Y.Z` tag is pushed.
To cut one:

```bash
scripts/release.sh patch       # e.g. 0.1.0 -> 0.1.1: bump Cargo.toml, tag, push -> CI publishes
scripts/release.sh minor       # 0.1.0 -> 0.2.0
scripts/release.sh major       # 0.1.0 -> 1.0.0
DRY_RUN=1 scripts/release.sh patch   # everything except the push
```

## What's inside / limitations

- `.anki/` holds `collection.anki2` (a regular SQLite DB with Anki's schema — openable in
  desktop Anki), `config.json`, `collection.media/`, and a `.gitignore` with `*` so none of it
  accidentally lands in git.
- The session key (hkey) is stored in `.anki/config.json` in the clear (mode 0600) — same as
  desktop Anki. The password is not stored. `logout` erases the key.
- AnkiWeb redirects to a shard (e.g. `sync11.ankiweb.net`) — the CLI picks that up and
  remembers the endpoint.
- Media file sync is **not implemented yet** (images/audio in notes sync as text references;
  the files themselves don't).
- Card study (scheduler/review) isn't exposed in the CLI — the assumption is that you study in
  regular Anki, while the CLI is for authoring and syncing.
- License: `rslib` is AGPL-3.0, so this tool is AGPL-3.0 too.
