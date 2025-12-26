#!/bin/bash
# Bump version across all version touchpoints
# Usage: ./scripts/bump-version.sh 0.1.0-beta.30

set -e

show_help() {
    echo "Usage: $0 <version> [--cargo]"
    echo "Example: $0 0.1.0-beta.30"
    echo ""
    echo "Updates version in:"
    echo "  - plugins/claude-code/.claude-plugin/plugin.json"
    echo "  - plugins/claude-code/bin/rkt-wrapper.sh"
    echo ""
    echo "Options:"
    echo "  --cargo  Also update Cargo.toml (usually only for base version changes)"
    exit 0
}

if [ -z "$1" ] || [ "$1" = "-h" ] || [ "$1" = "--help" ]; then
    show_help
fi

# Validate version format (X.Y.Z or X.Y.Z-suffix)
if ! echo "$1" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
    echo "Error: Invalid version format: $1"
    echo "Expected format: X.Y.Z or X.Y.Z-beta.N"
    exit 1
fi

NEW_VERSION="$1"
UPDATE_CARGO=false
if [ "$2" = "--cargo" ]; then
    UPDATE_CARGO=true
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

echo "Bumping to version: $NEW_VERSION"

# 1. Optionally update Cargo.toml (workspace root)
if [ "$UPDATE_CARGO" = true ]; then
    echo "  Updating Cargo.toml..."
    sed -i '' "s/^version = \"[^\"]*\"/version = \"$NEW_VERSION\"/" "$ROOT_DIR/Cargo.toml"
fi

# 2. Update Claude plugin.json
PLUGIN_JSON="$ROOT_DIR/plugins/claude-code/.claude-plugin/plugin.json"
echo "  Updating plugin.json..."
sed -i '' "s/\"version\": \"[^\"]*\"/\"version\": \"$NEW_VERSION\"/" "$PLUGIN_JSON"

# 3. Update rkt-wrapper.sh
WRAPPER_SH="$ROOT_DIR/plugins/claude-code/bin/rkt-wrapper.sh"
echo "  Updating rkt-wrapper.sh..."
sed -i '' "s/^VERSION=\"[^\"]*\"/VERSION=\"$NEW_VERSION\"/" "$WRAPPER_SH"

echo ""
echo "Version bumped to $NEW_VERSION in:"
if [ "$UPDATE_CARGO" = true ]; then
    echo "  - Cargo.toml"
fi
echo "  - plugins/claude-code/.claude-plugin/plugin.json"
echo "  - plugins/claude-code/bin/rkt-wrapper.sh"
echo ""
echo "Next steps:"
echo "  1. git add -A && git commit -m 'chore: bump version to $NEW_VERSION'"
echo "  2. git tag v$NEW_VERSION"
echo "  3. git push && git push --tags"
echo ""
echo "The release workflow will then:"
echo "  - Build binaries for all platforms"
echo "  - Create GitHub release"
echo "  - Update Homebrew formula"
echo "  - Update Scoop manifest"
