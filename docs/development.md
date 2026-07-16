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
- Media file sync is **not implemented yet** (images/audio in notes sync as text references;
  the files themselves don't).
- Card study (scheduler/review) isn't exposed in the CLI — the assumption is that you study in
  regular Anki, while the CLI is for authoring and syncing.
- License: `rslib` is AGPL-3.0, so this tool is AGPL-3.0 too.
