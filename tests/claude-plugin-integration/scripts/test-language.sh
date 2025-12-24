#!/bin/bash
# Test a single language using Claude Code headless mode with Rocket Index plugin

set -e

LANG_DIR="$1"
if [ -z "$LANG_DIR" ]; then
    echo "Usage: ./test-language.sh <language-dir>"
    echo "Example: ./test-language.sh ruby-sample"
    exit 1
fi

REPO_PATH="test-repos/$LANG_DIR"
RESULT_FILE="results/${LANG_DIR}_test.json"

if [ ! -d "$REPO_PATH" ]; then
    echo "Error: $REPO_PATH not found"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
RESULT_FILE_ABS="$SCRIPT_DIR/results/${LANG_DIR}_test.json"
PLUGIN_DIR="$(cd "$SCRIPT_DIR/../../plugins/claude-code" && pwd)"

echo "Testing $LANG_DIR..."

# Index the project first
cd "$SCRIPT_DIR/$REPO_PATH"
echo "  Indexing..."
rkt index

# Run Claude Code in headless mode with the plugin
# The plugin's PreToolUse hook auto-injects cwd as project_root
echo "  Running Claude Code with plugin..."

claude -p "I'm testing the rocket-index plugin on a $LANG_DIR project.

Please use the available rocket-index MCP tools (prefer mcp__plugin_rocketindex_rocket-index__* tools) to:
1. Find the definition of 'User'
2. Find all callers of 'process_payment' (or 'processPayment')
3. Search for symbols matching 'Payment*'

For each task:
- State which tool you're using
- Show the result (file, line number, symbol name)
- Confirm if the result matches the actual code location

Output your findings as structured JSON." \
  --plugin-dir "$PLUGIN_DIR" \
  --output-format json \
  --dangerously-skip-permissions \
  --max-budget-usd 0.50 \
  > "$RESULT_FILE_ABS" 2>&1

echo "  ✓ Results saved to $RESULT_FILE_ABS"
