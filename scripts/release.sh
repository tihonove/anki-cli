#!/bin/sh
# Cut a release locally: bump the version, commit, tag vX.Y.Z, and push.
# Pushing the tag is what triggers CI to build and publish the binaries.
#
#   scripts/release.sh 0.2.0        # bump, tag, push -> CI releases
#   DRY_RUN=1 scripts/release.sh 0.2.0   # do everything except the push
set -eu

usage() { echo "usage: scripts/release.sh <version>   e.g. scripts/release.sh 0.2.0" >&2; exit 1; }
[ $# -eq 1 ] || usage

VERSION="${1#v}"
echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.]+)?$' \
  || { echo "error: '$1' is not a semver version (e.g. 0.2.0)" >&2; exit 1; }
TAG="v$VERSION"

cd "$(git rev-parse --show-toplevel)"

# Guards: clean tree, tag not taken.
[ -z "$(git status --porcelain)" ] \
  || { echo "error: working tree is dirty; commit or stash first" >&2; exit 1; }
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null 2>&1; then
  echo "error: tag $TAG already exists" >&2; exit 1
fi

current="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[ -n "$current" ] || { echo "error: could not read current version from Cargo.toml" >&2; exit 1; }
echo "Releasing $current -> $VERSION"

# Bump [package] version and sync Cargo.lock so CI's --locked stays happy.
sed -i.bak "s/^version = \"$current\"/version = \"$VERSION\"/" Cargo.toml && rm -f Cargo.toml.bak
cargo update -p anki-cli --offline >/dev/null 2>&1 || cargo update -p anki-cli

git add Cargo.toml Cargo.lock
git commit -m "Release $TAG"
git tag -a "$TAG" -m "$TAG"

if [ "${DRY_RUN:-}" = "1" ]; then
  echo "DRY_RUN: created commit + tag $TAG locally; skipping push."
  echo "Push manually with:  git push --follow-tags"
else
  git push --follow-tags
  echo "Pushed $TAG. Release build: https://github.com/tihonove/anki-cli/actions"
fi
