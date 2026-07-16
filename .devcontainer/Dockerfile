# Rust dev container for anki-cli.
# Base image ships a recent stable Rust via rustup, a non-root `vscode` user (uid 1000),
# and git / build-essential / pkg-config / lld preinstalled.
FROM mcr.microsoft.com/devcontainers/rust:1-bookworm

# --- system deps (root) --------------------------------------------------------
# TLS is rustls everywhere (reqwest rustls-tls-native-roots + anki `rustls` feature),
# so no libssl-dev is needed — only ca-certificates for the native root store.
RUN apt-get update \
    && apt-get install -y --no-install-recommends unzip ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

# --- protoc (Protocol Buffers compiler) ---------------------------------------
# anki_proto's build script requires protoc; CI pins 29.3. Install the matching
# release into /usr/local (protoc binary + well-known-type includes).
ARG PROTOC_VERSION=29.3
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
      amd64) protoc_arch="x86_64" ;; \
      arm64) protoc_arch="aarch_64" ;; \
      *) echo "unsupported arch: $arch" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /tmp/protoc.zip \
      "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-${protoc_arch}.zip"; \
    unzip -o /tmp/protoc.zip -d /usr/local bin/protoc 'include/*'; \
    rm /tmp/protoc.zip; \
    chmod +x /usr/local/bin/protoc; \
    protoc --version

# --- vexx (TUI editor from github.com/tihonove/vexx) ---------------------------
# Prebuilt Linux binary from the latest release.
ARG VEXX_VERSION=v0.2.0
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
      amd64) vexx_asset="vexx-linux-x64" ;; \
      *) echo "no vexx binary for arch: $arch" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /usr/local/bin/vexx \
      "https://github.com/tihonove/vexx/releases/download/${VEXX_VERSION}/${vexx_asset}"; \
    chmod +x /usr/local/bin/vexx

# --- user-scoped tooling (vscode) ---------------------------------------------
USER vscode
ENV PATH="/home/vscode/.local/bin:${PATH}"

# Make sure the Rust toolchain is the freshest stable and has the usual components.
RUN rustup update stable \
    && rustup default stable \
    && rustup component add clippy rustfmt

# Claude Code CLI via the official native installer (lands in ~/.local/bin).
RUN curl -fsSL https://claude.ai/install.sh | bash
