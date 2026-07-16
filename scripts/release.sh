#!/bin/sh
# Cut a release locally: bump the version, commit, tag vX.Y.Z, and push.
# Pushing the tag is what triggers CI to build and publish the binaries.
#
#   scripts/release.sh patch        # 0.1.0 -> 0.1.1, tag, push -> CI releases
#   scripts/release.sh minor        # 0.1.0 -> 0.2.0
#   scripts/release.sh major        # 0.1.0 -> 1.0.0
#   DRY_RUN=1 scripts/release.sh patch   # everything except the push
set -eu

usage() { echo "usage: scripts/release.sh <major|minor|patch>" >&2; exit 1; }
[ $# -eq 1 ] || usage

cd "$(git rev-parse --show-toplevel)"

current="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
echo "$current" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' \
  || { echo "error: current Cargo.toml version '$current' is not plain MAJOR.MINOR.PATCH" >&2; exit 1; }

# Split MAJOR.MINOR.PATCH with parameter expansion (no external tools).
major="${current%%.*}"
rest="${current#*.}"
minor="${rest%%.*}"
patch="${rest##*.}"

case "$1" in
  major) major=$((major + 1)); minor=0; patch=0 ;;
  minor) minor=$((minor + 1)); patch=0 ;;
  patch) patch=$((patch + 1)) ;;
  *) usage ;;
esac
VERSION="$major.$minor.$patch"
TAG="v$VERSION"

# Guards: clean tree, tag not taken.
[ -z "$(git status --porcelain)" ] \
  || { echo "error: working tree is dirty; commit or stash first" >&2; exit 1; }
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null 2>&1; then
  echo "error: tag $TAG already exists" >&2; exit 1
fi

echo "Releasing $current -> $VERSION ($1)"

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
