# Dev container (Rust, terminal-based)

A Rust dev environment for `anki-cli`, meant to be used from a **terminal** via the
[devcontainer CLI](https://github.com/devcontainers/cli). It bundles:

- fresh **stable Rust** (rustup) with `clippy` + `rustfmt`
- **protoc 29.3** (required by `anki_proto`'s build script)
- **`vexx`** — the TUI editor from <https://github.com/tihonove/vexx>
- **`claude`** — the Claude Code CLI
- **`tmux`** — terminal multiplexer

## Prerequisites (host)

- Docker Engine (installed).
- `NODE_OPTIONS=--use-system-ca` is set globally on this machine and **breaks the
  devcontainer CLI's bundled node** — always run devcontainer commands with it unset:
  `env -u NODE_OPTIONS devcontainer …`
- If your shell isn't in the `docker` group yet (before the next login), wrap the command
  with `sg docker -c '…'`.

## Start / enter

```bash
# build + start (from the repo root)
env -u NODE_OPTIONS devcontainer up --workspace-folder .

# open a shell inside
env -u NODE_OPTIONS devcontainer exec --workspace-folder . bash
```

A convenient alias:

```bash
alias dc='env -u NODE_OPTIONS devcontainer'
# dc up --workspace-folder .
# dc exec --workspace-folder . bash
```

## Building anki-cli

Nothing to provision — just build:

```bash
cargo build --locked
cargo test --locked
```

`anki-cli` depends on the Anki Rust core (`anki` / `anki_proto`) via a **pinned git
dependency** in `Cargo.toml`, so cargo fetches the source and its i18n submodules itself.
`PROTOC` is already exported to `/usr/local/bin/protoc` in the container (needed by
`anki_proto`'s build script), overriding the user-specific path in `../.cargo/config.toml`.
TLS is rustls-only, so no OpenSSL is needed. The downloaded crate/git sources are cached in
named volumes, so rebuilds don't re-fetch them.
