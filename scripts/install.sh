#!/bin/sh
set -eu

REPO="endrico-fn/musick"
INSTALL_DIR="${MUSICK_INSTALL_DIR:-$HOME/.local/bin}"
FAST=0
VERSION=""

while [ $# -gt 0 ]; do
    case "$1" in
        --fast)
            FAST=1
            shift
            ;;
        --version)
            VERSION="${2:-}"
            shift 2
            ;;
        --dir)
            INSTALL_DIR="${2:-}"
            shift 2
            ;;
        -h|--help)
            cat <<'EOF'
Usage: install.sh [--fast] [--version vX.Y.Z] [--dir PATH]

Options:
  --fast            Install fast-resolver build (with GPL-3.0 rustypipe)
  --version vX.Y.Z  Install a specific release tag (default: latest)
  --dir PATH        Destination directory (default: ~/.local/bin)
EOF
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

OS="$(uname -s)"
if [ "$OS" != "Linux" ]; then
    echo "Unsupported OS: $OS (prebuilt binaries are Linux-only)" >&2
    exit 1
fi

ARCH="$(uname -m)"
case "$ARCH" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *)
        echo "Unsupported architecture: $ARCH" >&2
        exit 1
        ;;
esac

if [ "$FAST" -eq 1 ]; then
    ASSET="musick-fast-linux-${ARCH}.tar.gz"
else
    ASSET="musick-linux-${ARCH}.tar.gz"
fi

if [ -z "$VERSION" ]; then
    BASE_URL="https://github.com/${REPO}/releases/latest/download"
else
    BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Downloading ${ASSET}..."
curl -fsSL "${BASE_URL}/${ASSET}" -o "${TMP_DIR}/${ASSET}"
curl -fsSL "${BASE_URL}/SHA256SUMS.txt" -o "${TMP_DIR}/SHA256SUMS.txt"

EXPECTED="$(awk -v file="$ASSET" '$2 == file || $2 == "*"file { print $1 }' "${TMP_DIR}/SHA256SUMS.txt")"
if [ -z "$EXPECTED" ]; then
    echo "Checksum for ${ASSET} not found in SHA256SUMS.txt" >&2
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL="$(sha256sum "${TMP_DIR}/${ASSET}" | awk '{ print $1 }')"
elif command -v shasum >/dev/null 2>&1; then
    ACTUAL="$(shasum -a 256 "${TMP_DIR}/${ASSET}" | awk '{ print $1 }')"
else
    echo "Neither sha256sum nor shasum found to verify download" >&2
    exit 1
fi

if [ "$ACTUAL" != "$EXPECTED" ]; then
    echo "SHA-256 verification failed!" >&2
    echo "  expected: $EXPECTED" >&2
    echo "  actual:   $ACTUAL" >&2
    exit 1
fi

mkdir -p "$INSTALL_DIR"
tar -xzf "${TMP_DIR}/${ASSET}" -C "$TMP_DIR"
install -m 755 "${TMP_DIR}/musick" "${INSTALL_DIR}/musick"

echo "Installed musick to ${INSTALL_DIR}/musick"

for dep in mpv yt-dlp; do
    if ! command -v "$dep" >/dev/null 2>&1; then
        echo "Note: required runtime dependency '$dep' not found in PATH."
    fi
done
if ! command -v cava >/dev/null 2>&1; then
    echo "Tip: install optional 'cava' for the live audio spectrum visualizer."
fi

case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        echo "Add ${INSTALL_DIR} to your PATH to run 'musick' directly."
        ;;
esac
