#!/usr/bin/env sh
# Install the Girder CLI (`girder`) from a GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/dhishwasher/Girder/main/install.sh | sh
#
# Environment:
#   GIRDER_VERSION   tag to install (default: latest release)
#   GIRDER_BIN_DIR   install directory (default: ~/.local/bin)
#   GIRDER_REPO      owner/name to download from
#   GIRDER_BASE_URL  download host, for an internal mirror or an air-gapped
#                     network that cannot reach github.com. Assets must sit at
#                     <base>/<version>/<asset>.
#   GIRDER_SKIP_CHECKSUM=1  install without verifying the download. Only for
#                     a mirror that does not carry the .sha256 files.
#
# POSIX sh on purpose: this has to run under dash and busybox ash, not just
# bash. Every failure exits non-zero with a message on stderr, because a
# half-installed binary on PATH is worse than no binary.
set -eu

REPO="${GIRDER_REPO:-dhishwasher/Girder}"
BIN_DIR="${GIRDER_BIN_DIR:-$HOME/.local/bin}"

die() {
    echo "install.sh: $*" >&2
    exit 1
}

need() {
    command -v "$1" > /dev/null 2>&1 || die "requires $1"
}

need uname
need mkdir
need tar

# The release publishes a .sha256 beside every asset, so verification is the
# normal path and not a bonus. Resolve the hashing tool up front, next to the
# other hard requirements, so a host that cannot verify says so before it
# downloads anything rather than after.
if [ "${GIRDER_SKIP_CHECKSUM:-}" != 1 ]; then
    if command -v sha256sum > /dev/null 2>&1; then
        sha256_of() { sha256sum "$1" | cut -d ' ' -f 1; }
    elif command -v shasum > /dev/null 2>&1; then
        sha256_of() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
    else
        die "requires sha256sum or shasum to verify the download; install either one, or set GIRDER_SKIP_CHECKSUM=1 to install unverified"
    fi
fi

if command -v curl > /dev/null 2>&1; then
    fetch() { curl -fsSL "$1"; }
    fetch_to() { curl -fsSL "$1" -o "$2"; }
elif command -v wget > /dev/null 2>&1; then
    fetch() { wget -qO- "$1"; }
    fetch_to() { wget -qO "$2" "$1"; }
else
    die "requires curl or wget"
fi

# Release assets are named by Rust target triple, so map the host to one.
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
    Linux) os_part="unknown-linux-musl" ;;
    Darwin) os_part="apple-darwin" ;;
    *) die "unsupported OS: $os (build from source: cargo install --path crates/aether-app)" ;;
esac
case "$arch" in
    x86_64 | amd64) arch_part="x86_64" ;;
    arm64 | aarch64) arch_part="aarch64" ;;
    *) die "unsupported architecture: $arch" ;;
esac
# No aarch64 Linux binary is published yet; say so rather than 404ing.
if [ "$os_part" = "unknown-linux-musl" ] && [ "$arch_part" = "aarch64" ]; then
    die "no prebuilt aarch64 Linux binary yet (build from source: cargo install --path crates/aether-app)"
fi
target="${arch_part}-${os_part}"

version="${GIRDER_VERSION:-}"
if [ -z "$version" ]; then
    echo "Resolving latest release of $REPO ..." >&2
    # Deliberately no jq dependency.
    version="$(
        fetch "https://api.github.com/repos/$REPO/releases/latest" \
            | tr ',' '\n' \
            | grep '"tag_name"' \
            | head -n 1 \
            | cut -d '"' -f 4
    )" || true
    [ -n "$version" ] || die "could not resolve the latest release; set GIRDER_VERSION=vX.Y.Z"
fi

asset="girder-${target}.tar.gz"
base_url="${GIRDER_BASE_URL:-https://github.com/$REPO/releases/download}"
url="$base_url/$version/$asset"

tmp="$(mktemp -d)"
# shellcheck disable=SC2064 # $tmp must expand now, not at trap time.
trap "rm -rf '$tmp'" EXIT INT TERM

echo "Downloading $asset ($version) ..." >&2
fetch_to "$url" "$tmp/$asset" || die "download failed: $url"

# This script is piped into a shell straight off the network, so the checksum
# is the only thing that makes the download self-verifying. Every way of not
# verifying it is therefore fatal unless the operator asked for that: a
# checksum that will not fetch means a broken release, a mirror that carries
# no .sha256, or someone interfering with that one request — and the third is
# the case verification exists to catch. Warning and installing anyway, as
# this did before, let a single blocked request silently downgrade any install
# to unverified.
if [ "${GIRDER_SKIP_CHECKSUM:-}" = 1 ]; then
    echo "warning: GIRDER_SKIP_CHECKSUM=1, so $asset was not verified" >&2
else
    fetch_to "$url.sha256" "$tmp/$asset.sha256" 2> /dev/null || die "no checksum at $url.sha256; refusing to install $asset unverified (set GIRDER_SKIP_CHECKSUM=1 to override)"
    expected="$(cut -d ' ' -f 1 < "$tmp/$asset.sha256")"
    # A mirror can answer 200 with an error page. Reporting that as a mismatch
    # would send the user looking at their download instead of their mirror,
    # so require the shape of a checksum: 64 hex digits, nothing else.
    case "$expected" in
        *[!0-9a-fA-F]*) die "malformed checksum file at $url.sha256" ;;
    esac
    [ "${#expected}" -eq 64 ] || die "malformed checksum file at $url.sha256"
    actual="$(sha256_of "$tmp/$asset")"
    [ "$actual" = "$expected" ] || die "checksum mismatch for $asset (expected $expected, got $actual)"
fi

tar xzf "$tmp/$asset" -C "$tmp" || die "could not unpack $asset"
[ -f "$tmp/girder" ] || die "archive did not contain a girder binary"

mkdir -p "$BIN_DIR"
# Install via a temporary name and rename, so an interrupted copy can never
# leave a truncated executable where a working one used to be.
cp "$tmp/girder" "$BIN_DIR/.girder.incoming"
chmod +x "$BIN_DIR/.girder.incoming"
mv "$BIN_DIR/.girder.incoming" "$BIN_DIR/girder"

echo "Installed $("$BIN_DIR/girder" --version) to $BIN_DIR/girder" >&2

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        echo "" >&2
        echo "$BIN_DIR is not on your PATH. Add it:" >&2
        echo "  export PATH=\"$BIN_DIR:\$PATH\"" >&2
        ;;
esac

cat >&2 <<'NEXT'

Next: point your coding agent at it. For Claude Code:

  claude mcp add girder -- girder mcp .

Or add to any MCP client's config:

  {"mcpServers": {"girder": {"command": "girder", "args": ["mcp", "."]}}}

Then `girder analyze .` to see the graph it builds.
NEXT
