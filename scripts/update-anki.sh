#!/bin/sh
# Re-pin the anki/rslib git dependency to a new commit or release tag.
#
#   scripts/update-anki.sh 25.02             # resolve a tag to its commit, re-pin
#   scripts/update-anki.sh <40-hex-sha>      # pin an explicit commit
#
# Rewrites the rev in BOTH the `anki` and `anki_proto` lines of Cargo.toml (they
# share one rev) and refreshes Cargo.lock. Does NOT build or test — that is what
# CI on the resulting PR is for. Idempotent: a no-op if already on that commit.
set -eu

REPO="https://github.com/ankitects/anki"

usage() { echo "usage: scripts/update-anki.sh <rev|tag>" >&2; exit 1; }
[ $# -eq 1 ] || usage

cd "$(git rev-parse --show-toplevel)"

ref="$1"

# A 40-char hex string is taken as a commit SHA; anything else is treated as a
# tag and resolved to the commit it points at. `^{}` dereferences an annotated
# tag object down to its commit; the bare ref is a fallback for lightweight tags.
if echo "$ref" | grep -Eq '^[0-9a-f]{40}$'; then
  new="$ref"
else
  new="$(git ls-remote "$REPO" "refs/tags/$ref^{}" "refs/tags/$ref" | head -1 | cut -f1)"
  [ -n "$new" ] || { echo "error: tag '$ref' not found in $REPO" >&2; exit 1; }
fi

current="$(sed -n 's/.*rev = "\([0-9a-f]\{40\}\)".*/\1/p' Cargo.toml | head -1)"
[ -n "$current" ] || { echo "error: no rev = \"<sha>\" found in Cargo.toml" >&2; exit 1; }

if [ "$current" = "$new" ]; then
  echo "already pinned to $new; nothing to do"
  exit 0
fi

echo "Re-pinning anki: $current -> $new"

# Global flag rewrites both the `anki` and `anki_proto` rev fields in one pass.
sed -i.bak "s/rev = \"$current\"/rev = \"$new\"/g" Cargo.toml && rm -f Cargo.toml.bak

# Refresh Cargo.lock for the two crates only (no compilation, so no protoc needed).
cargo update -p anki -p anki_proto

echo "Done. Compare: $REPO/compare/$current...$new"
