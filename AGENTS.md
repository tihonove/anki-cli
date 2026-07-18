# AGENTS.md

Guidance for AI agents (and humans) working in this repository.

## Commit messages — Conventional Commits

Use [Conventional Commits](https://www.conventionalcommits.org/). The subject line must be:

```
<type>(<optional scope>): <short imperative summary>
```

Release notes are generated from history (`generate_release_notes` in CI), so a consistent
commit format keeps the changelog readable.

**Types**

| type | when |
|------|------|
| `feat` | a new user-facing capability |
| `fix` | a bug fix |
| `docs` | documentation only (README, this file, `docs/`) |
| `refactor` | code change that neither fixes a bug nor adds a feature |
| `perf` | performance improvement |
| `test` | adding or fixing tests only |
| `build` | build system, dependencies, `Cargo.toml`/`Cargo.lock` |
| `ci` | CI config (`.github/workflows/*`) |
| `chore` | maintenance that doesn't fit above (e.g. `Release v1.2.3`) |

**Scopes** (optional, use when it clarifies): `sync`, `media`, `mcp`, `cli`, `notes`, `config`.

**Rules**
- Subject in the imperative mood, lower-case, no trailing period, ~≤72 chars.
- Body (optional, after a blank line) explains *why*, not *what*.
- Breaking changes: add `!` before the colon (`feat(sync)!: …`) and/or a `BREAKING CHANGE:`
  footer.

**Examples**
```
feat(media): add sync-media for image/audio files
fix(sync): resolve the shard endpoint before media upload
ci: resolve protoc on the Windows e2e runner
docs: reflect media sync support in the README
chore: release v1.0.0
```

## Before committing

- `cargo test --locked` and `cargo clippy --all-targets` must pass.
- The AnkiWeb e2e test (`tests/e2e_ankiweb.rs`) self-skips without `ANKI_TEST_*` creds, so a
  plain `cargo test` stays offline and hermetic.
