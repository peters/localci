#!/bin/sh
# Install localci from the latest GitHub release.
#   curl -fsSL https://raw.githubusercontent.com/peters/localci/main/install.sh | sh
# LOCALCI_INSTALL_DIR picks the folder (default ~/.local/bin). LOCALCI_VERSION picks a tag.
set -eu

repo="peters/localci"
dir="${LOCALCI_INSTALL_DIR:-$HOME/.local/bin}"
version="${LOCALCI_VERSION:-latest}"

case "$(uname -s)" in
  Linux) os=unknown-linux-musl ;;
  Darwin) os=apple-darwin ;;
  *) echo "localci: unsupported system $(uname -s); on Windows use install.ps1" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=aarch64 ;;
  *) echo "localci: unsupported CPU $(uname -m)" >&2; exit 1 ;;
esac
target="$arch-$os"
asset="localci-$target.tar.gz"
if [ "$version" = latest ]; then
  # The newest release, pre-releases included; GitHub's "latest" link skips those.
  version="$(curl -fsSL "https://api.github.com/repos/$repo/releases?per_page=1" |
    sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
  [ -n "$version" ] || { echo "localci: no release found for $repo" >&2; exit 1; }
fi
base="https://github.com/$repo/releases/download/$version"

work="$(mktemp -d "${TMPDIR:-/tmp}/localci.XXXXXX")"
trap 'rm -rf "$work"' EXIT
echo "localci: downloading $asset"
curl -fsSL "$base/$asset" -o "$work/$asset"
curl -fsSL "$base/SHA256SUMS" -o "$work/SHA256SUMS"
expected="$(grep " $asset\$" "$work/SHA256SUMS" | cut -d' ' -f1)"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$work/$asset" | cut -d' ' -f1)"
else
  actual="$(shasum -a 256 "$work/$asset" | cut -d' ' -f1)"
fi
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
  echo "localci: checksum mismatch for $asset" >&2
  exit 1
fi
tar -xzf "$work/$asset" -C "$work"
mkdir -p "$dir"
mv "$work/localci" "$dir/localci"
chmod +x "$dir/localci"
echo "localci: installed $("$dir/localci" --version) to $dir/localci"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "localci: add $dir to your PATH, for example: export PATH=\"$dir:\$PATH\"" ;;
esac
echo "localci: next, run 'localci skill install' to teach your coding agent"
