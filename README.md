# anki-cli

**A standalone command-line Anki — and an MCP server for agents.** A single
self-contained binary that keeps a local copy of your collection, lets you edit it from the
terminal (or hand it to an agent), and syncs with AnkiWeb.

It talks to AnkiWeb **directly**, over Anki's real sync protocol — built on the official
`rslib` (Anki's Rust core, [ankitects/anki](https://github.com/ankitects/anki)), so the
database, schema, and sync are exactly the same as real Anki.

- **Standalone** — no Anki desktop, no AnkiConnect, no running app. Just the binary.
- **CLI or MCP** — the same binary is a terminal tool and an MCP server (`anki-cli mcp`).
- **Git-like workflow** — a local collection you edit offline, plus explicit `pull` / `push`
  / `sync` with conflict resolution.
- **One-command install** — a prebuilt binary, no toolchain required.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/tihonove/anki-cli/main/install.sh | sh
```

Drops a prebuilt binary into `~/.local/bin` (override the dir with `ANKI_CLI_BIN`, or pin a
release with `ANKI_CLI_VERSION=vX.Y.Z`). Prebuilt binaries exist for **Linux x86_64** and
**macOS arm64**; you can also grab one by hand from the
[releases](https://github.com/tihonove/anki-cli/releases) (`chmod +x`, put it on your
`PATH`). On other platforms, [build from source](docs/development.md).

## Use it from your terminal

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

## Use it from an agent (MCP)

The same binary is an MCP server over stdio — nothing extra to install. Wire it into Claude
Code from a directory that already has an `.anki/` (run `anki-cli init` there first):

```bash
cd ~/lang/deutsch          # a directory with .anki (init already done)
claude mcp add anki -- anki-cli mcp
```

The server looks for `.anki/` upward from its working directory. Claude Code starts MCP
servers in the project directory, so **each project automatically uses its own collection and
its own account** — no global state to juggle.

**Auth.** Call the `anki_login` tool, or run `anki-cli login` once beforehand. To keep the
password out of the conversation, `anki_login` falls back to the server's `ANKI_USERNAME` /
`ANKI_PASSWORD` environment variables when its arguments are omitted. Either way only the
session key is stored, never the password.

**Tools** (14):

- **Auth** — `anki_login`, `anki_logout`
- **Sync** — `anki_status`, `anki_sync`, `anki_pull`, `anki_push`
- **Notes** — `anki_add_note`, `anki_add_notes` (bulk), `anki_search`, `anki_get_note`,
  `anki_edit_note`, `anki_delete_notes`
- **Schema** — `anki_list_decks`, `anki_list_models`

A sync conflict reaches the agent as `result: "conflict"` with a hint, resolved by calling
`anki_pull` (take server) or `anki_push` (take local) — see [Sync model](#sync-model) below.

A typical agent work loop from the CLI (same shape as the MCP tools):

```bash
anki-cli pull
anki-cli --json search "deck:Spanish tag:verb"       # what's already there
anki-cli --json add -d Spanish "el perro" "dog" -t noun
anki-cli sync || {
  # exit 2: the collections diverged — resolve in favour of the local version
  anki-cli push
}
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
mcp                                           run as an MCP server over stdio
```

Global flags:

- `--json` — machine-readable output (for agents); errors go to stderr as JSON.
- `--dir PATH` — point at the data directory explicitly, skipping the `.anki/` search.
  Priority: `--dir` > `$ANKI_CLI_HOME` > nearest `.anki/` up the tree; if none, an error with
  a hint about `init`.

## Sync model

Sync works like git — a local collection, and explicit commands to reconcile it with the
server:

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

## Scope & limitations

- `.anki/` holds `collection.anki2` — a regular SQLite DB with Anki's schema, openable in
  desktop Anki.
- The session key (hkey) is stored in `.anki/config.json` (mode 0600). The password is never
  stored; `logout` erases the key.
- Media file sync is **not implemented yet** (images/audio in notes sync as text references;
  the files themselves don't).
- Card study (scheduler/review) isn't exposed — the assumption is that you study in regular
  Anki, while this tool is for authoring and syncing.
- License: `rslib` is AGPL-3.0, so this tool is AGPL-3.0 too.

Building from source, releasing, and internals: see [docs/development.md](docs/development.md).
