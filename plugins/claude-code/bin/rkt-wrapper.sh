#!/bin/bash
# RocketIndex wrapper - downloads binary on first use
# This avoids bundling a 30MB binary in the plugin repo

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RKT_BIN="$SCRIPT_DIR/rkt"
VERSION="0.1.0-beta.33"

# Detect platform
case "$(uname -s)" in
    Darwin)
        case "$(uname -m)" in
            arm64) PLATFORM="aarch64-apple-darwin" ;;
            x86_64) PLATFORM="x86_64-apple-darwin" ;;
            *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
        esac
        ;;
    Linux)
        case "$(uname -m)" in
            x86_64) PLATFORM="x86_64-unknown-linux-gnu" ;;
            aarch64) PLATFORM="aarch64-unknown-linux-gnu" ;;
            *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
        esac
        ;;
    *)
        echo "Unsupported OS: $(uname -s)" >&2
        exit 1
        ;;
esac

# Check if we need to download (missing or wrong version)
NEED_DOWNLOAD=false
if [ ! -x "$RKT_BIN" ]; then
    NEED_DOWNLOAD=true
else
    # Check installed version matches expected version
    INSTALLED_VERSION=$("$RKT_BIN" --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?' || echo "unknown")
    if [ "$INSTALLED_VERSION" != "$VERSION" ]; then
        echo "Updating RocketIndex from $INSTALLED_VERSION to $VERSION..." >&2
        NEED_DOWNLOAD=true
    fi
fi

if [ "$NEED_DOWNLOAD" = true ]; then
    echo "Downloading RocketIndex $VERSION for $PLATFORM..." >&2

    DOWNLOAD_URL="https://github.com/rocket-tycoon/rocket-index/releases/download/v${VERSION}/rocketindex-v${VERSION}-${PLATFORM}.tar.gz"

    # Create temp directory
    TMP_DIR=$(mktemp -d)
    trap "rm -rf $TMP_DIR" EXIT

    # Download and extract
    if command -v curl &> /dev/null; then
        curl -fsSL "$DOWNLOAD_URL" -o "$TMP_DIR/rkt.tar.gz"
    elif command -v wget &> /dev/null; then
        wget -q "$DOWNLOAD_URL" -O "$TMP_DIR/rkt.tar.gz"
    else
        echo "Error: curl or wget required" >&2
        exit 1
    fi

    tar -xzf "$TMP_DIR/rkt.tar.gz" -C "$TMP_DIR"

    # Move binary to plugin bin directory
    mv "$TMP_DIR/rkt" "$RKT_BIN"
    chmod +x "$RKT_BIN"

    echo "RocketIndex installed successfully" >&2
fi

# Execute rkt with all arguments
exec "$RKT_BIN" "$@"
