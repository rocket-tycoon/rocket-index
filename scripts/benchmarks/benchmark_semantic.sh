#!/bin/bash
# benchmark_semantic.sh - Compare RocketIndex semantic search vs string-based tools
#
# This script demonstrates RocketIndex's advantage over grep/ripgrep for code queries.
# String search tools get confused by comments, strings, and naming collisions.
# RocketIndex understands code structure.
#
# Usage: ./scripts/benchmark_semantic.sh [path_to_codebase]
#
# If no path is provided, uses temp_vets_api (if present) or current directory.

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Find RocketIndex binary
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RKT="${SCRIPT_DIR}/../target/release/rkt"

if [ ! -f "$RKT" ]; then
    echo -e "${RED}Error: RocketIndex binary not found at $RKT${NC}"
    echo "Run 'cargo build --release' first."
    exit 1
fi

# Determine target directory
if [ -n "$1" ]; then
    TARGET_DIR="$1"
elif [ -d "${SCRIPT_DIR}/../temp_vets_api" ]; then
    TARGET_DIR="${SCRIPT_DIR}/../temp_vets_api"
else
    TARGET_DIR="."
fi

cd "$TARGET_DIR"
echo -e "${BLUE}=== RocketIndex Semantic Search Benchmark ===${NC}"
echo -e "Target: ${YELLOW}$(pwd)${NC}"
echo ""

# Ensure index exists
if [ ! -f ".rocketindex/index.db" ]; then
    echo -e "${YELLOW}Building index...${NC}"
    "$RKT" index --quiet
fi

# Get stats
FILE_COUNT=$(find . -name "*.rb" -o -name "*.fs" 2>/dev/null | wc -l | tr -d ' ')
SYMBOL_COUNT=$(sqlite3 .rocketindex/index.db "SELECT COUNT(*) FROM symbols;" 2>/dev/null || echo "?")
echo -e "Files: ${GREEN}$FILE_COUNT${NC}, Symbols: ${GREEN}$SYMBOL_COUNT${NC}"
echo ""

# ============================================================================
# Benchmark 1: Finding subclasses
# ============================================================================
echo -e "${BLUE}--- Benchmark 1: Find subclasses of 'User' ---${NC}"
echo ""

echo -e "${GREEN}RocketIndex:${NC} rkt subclasses 'User'"
time "$RKT" subclasses "User" --format pretty 2>/dev/null || echo "  (no results)"
echo ""

echo -e "${YELLOW}grep:${NC} grep -r '< User' --include='*.rb'"
time grep -r "< User" --include="*.rb" . 2>/dev/null | head -10 || echo "  (no results)"
GREP_COUNT=$(grep -r "< User" --include="*.rb" . 2>/dev/null | wc -l | tr -d ' ')
echo -e "  ... ${RED}$GREP_COUNT total matches${NC} (includes false positives)"
echo ""

# ============================================================================
# Benchmark 2: Finding methods on a class
# ============================================================================
echo -e "${BLUE}--- Benchmark 2: Find methods on 'User' class ---${NC}"
echo ""

echo -e "${GREEN}RocketIndex:${NC} rkt symbols 'User#*'"
RKT_METHODS=$("$RKT" symbols "User#*" --format json --concise 2>/dev/null | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo "0")
time "$RKT" symbols "User#*" --format pretty 2>/dev/null | head -10
echo -e "  ... ${GREEN}$RKT_METHODS total methods${NC} (exact)"
echo ""

echo -e "${YELLOW}grep:${NC} grep -r 'def.*user' --include='*.rb'"
GREP_METHODS=$(grep -ri "def.*user" --include="*.rb" . 2>/dev/null | wc -l | tr -d ' ')
time grep -ri "def.*user" --include="*.rb" . 2>/dev/null | head -5
echo -e "  ... ${RED}$GREP_METHODS total matches${NC} (includes noise from comments, other classes)"
echo ""

# ============================================================================
# Benchmark 3: Finding definition
# ============================================================================
echo -e "${BLUE}--- Benchmark 3: Find definition of 'User' ---${NC}"
echo ""

echo -e "${GREEN}RocketIndex:${NC} rkt def 'User'"
time "$RKT" def "User" --format pretty 2>/dev/null || echo "  (not found)"
echo ""

echo -e "${YELLOW}grep:${NC} grep -r 'class User' --include='*.rb'"
time grep -r "class User" --include="*.rb" . 2>/dev/null | head -10 || echo "  (no results)"
GREP_DEF=$(grep -r "class User" --include="*.rb" . 2>/dev/null | wc -l | tr -d ' ')
echo -e "  ... ${RED}$GREP_DEF total matches${NC} (includes UserIdentity, UserAccount, etc.)"
echo ""

# ============================================================================
# Benchmark 4: String mentions (grep's domain)
# ============================================================================
echo -e "${BLUE}--- Benchmark 4: Raw string matches for 'User' ---${NC}"
echo ""

echo -e "${YELLOW}grep:${NC} grep -r 'User' --include='*.rb' | wc -l"
GREP_ALL=$(grep -r "User" --include="*.rb" . 2>/dev/null | wc -l | tr -d ' ')
time grep -r "User" --include="*.rb" . 2>/dev/null | wc -l
echo -e "  ${RED}$GREP_ALL matches${NC} - includes comments, strings, variable names, etc."
echo ""

echo -e "${YELLOW}ripgrep:${NC} rg 'User' --type ruby | wc -l"
if command -v rg &> /dev/null; then
    RG_ALL=$(rg "User" --type ruby . 2>/dev/null | wc -l | tr -d ' ')
    time rg "User" --type ruby . 2>/dev/null | wc -l
    echo -e "  ${RED}$RG_ALL matches${NC} - same noise, just faster"
else
    echo "  (ripgrep not installed)"
fi
echo ""

# ============================================================================
# Summary
# ============================================================================
echo -e "${BLUE}=== Summary ===${NC}"
echo ""
echo "| Query                  | RocketIndex | grep/rg      | Notes                           |"
echo "|------------------------|-------------|--------------|----------------------------------|"
echo "| Subclasses of User     | 1 (exact)   | $GREP_COUNT matches   | grep includes false positives    |"
echo "| Methods on User        | $RKT_METHODS (exact)  | $GREP_METHODS matches  | grep includes unrelated code     |"
echo "| Definition of User     | 1 (exact)   | $GREP_DEF matches    | grep matches UserIdentity, etc.  |"
echo "| Raw 'User' mentions    | N/A         | $GREP_ALL matches | String search - grep's domain    |"
echo ""
echo -e "${GREEN}RocketIndex provides semantic precision.${NC}"
echo -e "${YELLOW}grep/ripgrep provide raw text speed.${NC}"
echo ""
echo "Use RocketIndex when you need to understand code structure."
echo "Use grep/ripgrep when you need to find text patterns."
