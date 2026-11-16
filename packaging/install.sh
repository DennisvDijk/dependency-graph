#!/bin/sh
# Install a prebuilt depgraph binary from GitHub releases.
# Falls back to instructions when no release asset exists yet.
set -eu
repo="${DEPGRAPH_REPO:-DennisvDijk/dependency-graph}"
os=$(uname -s | tr '[:upper:]' '[:lower:]')
arch=$(uname -m)
case "$os-$arch" in
  darwin-arm64) target=aarch64-apple-darwin ;;
  darwin-x86_64) target=x86_64-apple-darwin ;;
  linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  *)
    echo "no prebuilt binary for $os-$arch. next: cargo install --git https://github.com/${repo} --locked" >&2
    exit 1
    ;;
esac
version="${DEPGRAPH_VERSION:-latest}"
if [ "$version" = latest ]; then
  url="https://github.com/${repo}/releases/latest/download/depgraph-${target}.tar.gz"
else
  url="https://github.com/${repo}/releases/download/${version}/depgraph-${target}.tar.gz"
fi
dest="${HOME}/.local/bin"
mkdir -p "$dest"
tmp=$(mktemp -d)
if ! curl -fsSL "$url" | tar -xz -C "$tmp"; then
  echo "download failed. next: cargo install --git https://github.com/${repo} --locked" >&2
  exit 1
fi
install -m 0755 "$tmp/depgraph" "$dest/depgraph"
echo "installed $dest/depgraph"
echo "next: depgraph init && depgraph doctor"
