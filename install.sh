#!/bin/sh
# anki-cli installer.
#
#   curl -fsSL https://raw.githubusercontent.com/tihonove/anki-cli/main/install.sh | sh
#
# Downloads the prebuilt binary for your OS/arch from GitHub Releases and drops it
# into a bin directory. Overridable via environment:
#   ANKI_CLI_VERSION   release tag to install (default: latest), e.g. v0.2.0
#   ANKI_CLI_BIN       install directory (default: $HOME/.local/bin)
set -eu

REPO="tihonove/anki-cli"
VERSION="${ANKI_CLI_VERSION:-latest}"
BIN_DIR="${ANKI_CLI_BIN:-$HOME/.local/bin}"

err() { printf 'error: %s\n' "$1" >&2; exit 1; }

# Map uname -> the release asset name (must match the names produced by CI).
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux)  os_part="linux" ;;
  Darwin) os_part="macos" ;;
  *) err "unsupported OS: $os (only Linux and macOS have prebuilt binaries)" ;;
esac
case "$arch" in
  x86_64|amd64)  arch_part="x86_64" ;;
  arm64|aarch64) arch_part="aarch64" ;;
  *) err "unsupported architecture: $arch" ;;
esac
asset="anki-cli-${os_part}-${arch_part}"

# Only the combinations CI actually builds exist as assets.
case "$asset" in
  anki-cli-linux-x86_64|anki-cli-macos-aarch64) ;;
  *) err "no prebuilt binary for ${os}/${arch} yet — build from source instead" ;;
esac

if [ "$VERSION" = "latest" ]; then
  url="https://github.com/${REPO}/releases/latest/download/${asset}"
else
  url="https://github.com/${REPO}/releases/download/${VERSION}/${asset}"
fi

command -v curl >/dev/null 2>&1 || err "curl is required"

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT INT TERM

printf 'Downloading %s (%s)...\n' "$asset" "$VERSION"
curl -fSL --proto '=https' -o "$tmp" "$url" \
  || err "download failed: $url"

mkdir -p "$BIN_DIR"
target="${BIN_DIR}/anki-cli"
chmod +x "$tmp"
# Clear the quarantine flag so macOS doesn't block the unsigned binary.
[ "$os" = "Darwin" ] && xattr -d com.apple.quarantine "$tmp" 2>/dev/null || true
mv "$tmp" "$target"
trap - EXIT INT TERM

printf 'Installed %s\n' "$("$target" --version 2>/dev/null || echo "anki-cli -> $target")"

case ":${PATH}:" in
  *":${BIN_DIR}:"*) ;;
  *) printf '\nNote: %s is not on your PATH. Add it, e.g.:\n  export PATH="%s:$PATH"\n' \
       "$BIN_DIR" "$BIN_DIR" ;;
esac
