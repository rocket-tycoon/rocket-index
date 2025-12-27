#!/bin/bash
# Test RocketIndex commands and MCP tools against real-world repositories
#
# Usage: ./scripts/test-real-repos.sh [OPTIONS]
#
# Options:
#   --repo <lang/name>  Test only a specific repo (e.g., "python/django")
#   --skip-index        Skip indexing (use existing index)
#   --quick             Skip large repos (>500 files)
#   --verbose, -v       Enable verbose logging
#
# Output:
#   - Console: Colored test results
#   - JSON: results/test-real-repos-<timestamp>.json
#   - Markdown: results/test-real-repos-<timestamp>.md

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
RKT="$ROOT_DIR/target/release/rkt"
CONFIG_FILE="$SCRIPT_DIR/test-real-repos-config.json"
TEST_REPOS_DIR="$ROOT_DIR/test-repos"
RESULTS_DIR="$ROOT_DIR/results"

# Parse arguments
SINGLE_REPO=""
SKIP_INDEX=false
QUICK_MODE=false
VERBOSE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --repo)
            SINGLE_REPO="$2"
            shift 2
            ;;
        --skip-index)
            SKIP_INDEX=true
            shift
            ;;
        --quick)
            QUICK_MODE=true
            shift
            ;;
        --verbose|-v)
            VERBOSE=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Counters
TOTAL_PASSED=0
TOTAL_FAILED=0
TOTAL_SKIPPED=0

# Results arrays (JSON will be built incrementally)
RESULTS_JSON="[]"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Ensure binary exists
if [ ! -x "$RKT" ]; then
    echo -e "${RED}Error: rkt binary not found at $RKT${NC}"
    echo "Run: cargo build --release"
    exit 1
fi

# Ensure config exists
if [ ! -f "$CONFIG_FILE" ]; then
    echo -e "${RED}Error: Config file not found at $CONFIG_FILE${NC}"
    exit 1
fi

# Create results directory
mkdir -p "$RESULTS_DIR"

# Helper: Add result to JSON array
add_result() {
    local repo="$1"
    local test_name="$2"
    local status="$3"
    local rkt_result="$4"
    local grep_result="$5"
    local rkt_time="$6"
    local grep_time="$7"
    local notes="$8"

    local result=$(jq -n \
        --arg repo "$repo" \
        --arg test "$test_name" \
        --arg status "$status" \
        --arg rkt_result "$rkt_result" \
        --arg grep_result "$grep_result" \
        --arg rkt_time "$rkt_time" \
        --arg grep_time "$grep_time" \
        --arg notes "$notes" \
        '{repo: $repo, test: $test, status: $status, rkt_result: $rkt_result, grep_result: $grep_result, rkt_time_ms: $rkt_time, grep_time_ms: $grep_time, notes: $notes}')

    RESULTS_JSON=$(echo "$RESULTS_JSON" | jq ". + [$result]")
}

# Helper: Get current time in milliseconds (cross-platform)
get_time_ms() {
    python3 -c "import time; print(int(time.time()*1000))"
}

# Verbose logging
log_verbose() {
    if [ "$VERBOSE" = true ]; then
        echo -e "${YELLOW}[VERBOSE]${NC} $1" >&2
    fi
}

# Helper: Run command with timing (returns ms)
time_ms() {
    local start=$(get_time_ms)
    eval "$@" > /dev/null 2>&1
    local end=$(get_time_ms)
    echo $((end - start))
}

# Helper: Test pass/fail
test_pass() {
    local test_name="$1"
    echo -e "  ${GREEN}PASS${NC} $test_name"
    TOTAL_PASSED=$((TOTAL_PASSED + 1))
}

test_fail() {
    local test_name="$1"
    local reason="$2"
    echo -e "  ${RED}FAIL${NC} $test_name: $reason"
    TOTAL_FAILED=$((TOTAL_FAILED + 1))
}

test_skip() {
    local test_name="$1"
    local reason="$2"
    echo -e "  ${YELLOW}SKIP${NC} $test_name: $reason"
    TOTAL_SKIPPED=$((TOTAL_SKIPPED + 1))
}

# Get file extension for language
get_extension() {
    case $1 in
        c) echo "c" ;;
        cpp) echo "cpp" ;;
        csharp) echo "cs" ;;
        fsharp) echo "fs" ;;
        go) echo "go" ;;
        java) echo "java" ;;
        javascript) echo "js" ;;
        kotlin) echo "kt" ;;
        php) echo "php" ;;
        python) echo "py" ;;
        ruby) echo "rb" ;;
        rust) echo "rs" ;;
        swift) echo "swift" ;;
        typescript) echo "ts" ;;
        *) echo "*" ;;
    esac
}

# Test a single repo
test_repo() {
    local repo_key="$1"
    local repo_path="$TEST_REPOS_DIR/$repo_key"

    if [ ! -d "$repo_path" ]; then
        echo -e "${YELLOW}Skipping $repo_key (not found)${NC}"
        return
    fi

    # Get config for this repo
    local config=$(jq -r ".[\"$repo_key\"]" "$CONFIG_FILE")
    if [ "$config" = "null" ]; then
        echo -e "${YELLOW}Skipping $repo_key (no config)${NC}"
        return
    fi

    local language=$(echo "$config" | jq -r '.language')
    local def_symbol=$(echo "$config" | jq -r '.def_symbol')
    local caller_symbol=$(echo "$config" | jq -r '.caller_symbol')
    local wildcard_pattern=$(echo "$config" | jq -r '.wildcard_pattern')
    local grep_def_pattern=$(echo "$config" | jq -r '.grep_def_pattern')
    local grep_caller_pattern=$(echo "$config" | jq -r '.grep_caller_pattern')
    local subclass_parent=$(echo "$config" | jq -r '.subclass_parent // empty')
    local ext=$(get_extension "$language")

    # Count files
    local file_count=$(find "$repo_path" -name "*.$ext" 2>/dev/null | wc -l | tr -d ' ')

    # Skip large repos in quick mode (>500 files can take minutes to index)
    if [ "$QUICK_MODE" = true ] && [ "$file_count" -gt 500 ]; then
        echo -e "${YELLOW}Skipping $repo_key (quick mode, $file_count files)${NC}"
        add_result "$repo_key" "skipped" "skip" "" "" "" "" "quick mode, $file_count files"
        TOTAL_SKIPPED=$((TOTAL_SKIPPED + 1))
        return
    fi

    echo ""
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}Testing: $repo_key${NC}"
    echo -e "${CYAN}Language: $language | Files: $file_count${NC}"
    echo -e "${BLUE}========================================${NC}"

    cd "$repo_path"

    # ========================================
    # 1. INDEX
    # ========================================
    local index_time=""
    local symbol_count=""
    local db_size=""

    if [ "$SKIP_INDEX" = true ] && [ -f ".rocketindex/index.db" ]; then
        echo -e "${YELLOW}Using existing index${NC}"
        symbol_count=$(sqlite3 .rocketindex/index.db "SELECT COUNT(*) FROM symbols;" 2>/dev/null || echo "?")
        db_size=$(du -h .rocketindex/index.db 2>/dev/null | cut -f1 || echo "?")
    else
        echo -e "Indexing..."
        local start=$(get_time_ms)
        local index_result=$("$RKT" index --quiet 2>&1 || echo '{"error": "index failed"}')
        local end=$(get_time_ms)
        index_time=$((end - start))

        if echo "$index_result" | jq -e '.symbols' > /dev/null 2>&1; then
            symbol_count=$(echo "$index_result" | jq '.symbols')
            db_size=$(du -h .rocketindex/index.db 2>/dev/null | cut -f1 || echo "?")
            echo -e "  ${GREEN}Indexed $symbol_count symbols in ${index_time}ms (DB: $db_size)${NC}"
            add_result "$repo_key" "index" "pass" "$symbol_count symbols" "" "$index_time" "" "db_size: $db_size"
        else
            echo -e "  ${RED}Index failed${NC}"
            add_result "$repo_key" "index" "fail" "" "" "$index_time" "" "index failed"
            ((TOTAL_FAILED++))
            return
        fi
    fi

    # ========================================
    # 2. FIND DEFINITION (rkt def)
    # ========================================
    echo ""
    echo "Testing: rkt def \"$def_symbol\""

    local rkt_start=$(get_time_ms)
    local def_result=$("$RKT" def "$def_symbol" --quiet 2>&1 || echo '{"error": "not found"}')
    local rkt_end=$(get_time_ms)
    local rkt_def_time=$((rkt_end - rkt_start))

    # Count RocketIndex results (1 = exact match)
    local rkt_def_count=0
    if echo "$def_result" | jq -e '.name' > /dev/null 2>&1; then
        rkt_def_count=1
    fi

    # Run grep equivalent
    local grep_start=$(get_time_ms)
    local grep_def_count=$(rg -c "$grep_def_pattern" --type "$language" . 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}' || echo "0")
    local grep_end=$(get_time_ms)
    local grep_def_time=$((grep_end - grep_start))

    log_verbose "Evaluating def result: count=$rkt_def_count"
    if [ "$rkt_def_count" -eq 1 ]; then
        local def_file=$(echo "$def_result" | jq -r '.file')
        local def_line=$(echo "$def_result" | jq -r '.line')
        log_verbose "Calling test_pass for def"
        test_pass "def \"$def_symbol\" -> $def_file:$def_line"
        log_verbose "Calling add_result for def"
        add_result "$repo_key" "def" "pass" "1 match" "$grep_def_count matches" "$rkt_def_time" "$grep_def_time" "file: $def_file:$def_line"
        log_verbose "add_result completed for def"
    elif [ "$rkt_def_count" -gt 1 ]; then
        test_fail "def \"$def_symbol\"" "ambiguous ($rkt_def_count matches)"
        add_result "$repo_key" "def" "fail" "$rkt_def_count matches" "$grep_def_count matches" "$rkt_def_time" "$grep_def_time" "ambiguous"
    else
        test_fail "def \"$def_symbol\"" "not found"
        add_result "$repo_key" "def" "fail" "0 matches" "$grep_def_count matches" "$rkt_def_time" "$grep_def_time" "not found"
    fi

    log_verbose "Printing comparison line"
    echo -e "  ${CYAN}Comparison: rkt=$rkt_def_count, grep=$grep_def_count (${rkt_def_time}ms vs ${grep_def_time}ms)${NC}"

    log_verbose "About to test callers"

    # ========================================
    # 3. FIND CALLERS (rkt callers)
    # ========================================
    echo ""
    echo "Testing: rkt callers \"$caller_symbol\""

    local rkt_start=$(get_time_ms)
    local callers_result=$("$RKT" callers "$caller_symbol" --quiet 2>&1 || echo '{"callers": []}')
    local rkt_end=$(get_time_ms)
    local rkt_callers_time=$((rkt_end - rkt_start))

    local rkt_callers_count=$(echo "$callers_result" | jq '.callers | length' 2>/dev/null || echo "0")

    # Run grep equivalent (text matches, not actual callers)
    local grep_start=$(get_time_ms)
    local grep_callers_count=$(rg -c "$grep_caller_pattern" --type "$language" . 2>/dev/null | awk -F: '{sum+=$2} END {print sum+0}' || echo "0")
    local grep_end=$(get_time_ms)
    local grep_callers_time=$((grep_end - grep_start))

    if [ "$rkt_callers_count" -gt 0 ]; then
        test_pass "callers \"$caller_symbol\" -> $rkt_callers_count callers"
        add_result "$repo_key" "callers" "pass" "$rkt_callers_count callers" "$grep_callers_count matches" "$rkt_callers_time" "$grep_callers_time" ""
    else
        test_skip "callers \"$caller_symbol\"" "no callers found (symbol may not be called)"
        add_result "$repo_key" "callers" "skip" "0 callers" "$grep_callers_count matches" "$rkt_callers_time" "$grep_callers_time" "no callers in index"
    fi

    echo -e "  ${CYAN}Comparison: rkt=$rkt_callers_count callers, grep=$grep_callers_count mentions (${rkt_callers_time}ms vs ${grep_callers_time}ms)${NC}"

    # ========================================
    # 4. FIND REFERENCES (rkt refs)
    # ========================================
    echo ""
    echo "Testing: rkt refs \"$def_symbol\""

    local rkt_start=$(get_time_ms)
    local refs_result=$("$RKT" refs "$def_symbol" --quiet 2>&1 || echo '[]')
    local rkt_end=$(get_time_ms)
    local rkt_refs_time=$((rkt_end - rkt_start))

    # refs returns a direct array, not {"references": [...]}
    local rkt_refs_count=$(echo "$refs_result" | jq 'if type == "array" then length else 0 end' 2>/dev/null || echo "0")

    if [ "$rkt_refs_count" -gt 0 ]; then
        test_pass "refs \"$def_symbol\" -> $rkt_refs_count references"
        add_result "$repo_key" "refs" "pass" "$rkt_refs_count refs" "" "$rkt_refs_time" "" ""
    else
        test_skip "refs \"$def_symbol\"" "no references found"
        add_result "$repo_key" "refs" "skip" "0 refs" "" "$rkt_refs_time" "" "no refs in index"
    fi

    # ========================================
    # 5. SPIDER (rkt spider)
    # ========================================
    echo ""
    echo "Testing: rkt spider \"$def_symbol\" --depth 1"

    local rkt_start=$(get_time_ms)
    # Use depth 1 to keep traversal fast
    local spider_result=$("$RKT" spider "$def_symbol" --depth 1 --quiet 2>&1 || echo '{"nodes": []}')
    local rkt_end=$(get_time_ms)
    local rkt_spider_time=$((rkt_end - rkt_start))

    local spider_nodes=$(echo "$spider_result" | jq '.nodes | length' 2>/dev/null || echo "0")

    if [ "$spider_nodes" -gt 0 ]; then
        test_pass "spider \"$def_symbol\" -> $spider_nodes nodes"
        add_result "$repo_key" "spider" "pass" "$spider_nodes nodes" "N/A" "$rkt_spider_time" "" "grep cannot do dependency graphs"
    else
        test_skip "spider \"$def_symbol\"" "no dependencies found"
        add_result "$repo_key" "spider" "skip" "0 nodes" "N/A" "$rkt_spider_time" "" "no deps"
    fi

    # ========================================
    # 6. REVERSE SPIDER (rkt spider --reverse)
    # ========================================
    echo ""
    echo "Testing: rkt spider \"$def_symbol\" --reverse --depth 1"

    local rkt_start=$(get_time_ms)
    # Use depth 1 to keep traversal fast
    local rev_spider_result=$("$RKT" spider "$def_symbol" --reverse --depth 1 --quiet 2>&1 || echo '{"nodes": []}')
    local rkt_end=$(get_time_ms)
    local rkt_rev_spider_time=$((rkt_end - rkt_start))

    local rev_spider_nodes=$(echo "$rev_spider_result" | jq '.nodes | length' 2>/dev/null || echo "0")

    if [ "$rev_spider_nodes" -gt 0 ]; then
        test_pass "spider --reverse \"$def_symbol\" -> $rev_spider_nodes nodes"
        add_result "$repo_key" "spider_reverse" "pass" "$rev_spider_nodes nodes" "N/A" "$rkt_rev_spider_time" "" "grep cannot do reverse dependency graphs"
    else
        test_skip "spider --reverse \"$def_symbol\"" "no callers found"
        add_result "$repo_key" "spider_reverse" "skip" "0 nodes" "N/A" "$rkt_rev_spider_time" "" "no reverse deps"
    fi

    # ========================================
    # 7. SYMBOL SEARCH (rkt symbols)
    # ========================================
    echo ""
    echo "Testing: rkt symbols \"$wildcard_pattern\""

    local rkt_start=$(get_time_ms)
    local symbols_result=$("$RKT" symbols "$wildcard_pattern" --quiet 2>&1 || echo '[]')
    local rkt_end=$(get_time_ms)
    local rkt_symbols_time=$((rkt_end - rkt_start))

    local symbols_count=$(echo "$symbols_result" | jq 'length' 2>/dev/null || echo "0")

    if [ "$symbols_count" -gt 0 ]; then
        test_pass "symbols \"$wildcard_pattern\" -> $symbols_count matches"
        add_result "$repo_key" "symbols" "pass" "$symbols_count matches" "" "$rkt_symbols_time" "" ""
    else
        test_fail "symbols \"$wildcard_pattern\"" "no matches"
        add_result "$repo_key" "symbols" "fail" "0 matches" "" "$rkt_symbols_time" "" "no symbol matches"
    fi

    # ========================================
    # 8. SUBCLASSES (rkt subclasses) - if applicable
    # ========================================
    if [ -n "$subclass_parent" ]; then
        echo ""
        echo "Testing: rkt subclasses \"$subclass_parent\""

        local rkt_start=$(get_time_ms)
        local subclasses_result=$("$RKT" subclasses "$subclass_parent" --quiet 2>&1 || echo '[]')
        local rkt_end=$(get_time_ms)
        local rkt_subclasses_time=$((rkt_end - rkt_start))

        local subclasses_count=$(echo "$subclasses_result" | jq 'if type == "array" then length else 0 end' 2>/dev/null || echo "0")
        subclasses_count=${subclasses_count//[^0-9]/}  # Strip non-numeric chars
        subclasses_count=${subclasses_count:-0}  # Default to 0 if empty

        if [ "$subclasses_count" -gt 0 ] 2>/dev/null; then
            test_pass "subclasses \"$subclass_parent\" -> $subclasses_count subclasses"
            add_result "$repo_key" "subclasses" "pass" "$subclasses_count subclasses" "N/A" "$rkt_subclasses_time" "" "grep cannot find subclasses semantically"
        else
            test_skip "subclasses \"$subclass_parent\"" "no subclasses found"
            add_result "$repo_key" "subclasses" "skip" "0 subclasses" "N/A" "$rkt_subclasses_time" "" "no subclasses in index"
        fi
    fi

    cd "$ROOT_DIR"
}

# ========================================
# MAIN
# ========================================

echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║     RocketIndex Real Repository Test Suite                 ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Binary: $RKT"
echo "Config: $CONFIG_FILE"
echo "Repos:  $TEST_REPOS_DIR"
echo ""

if [ -n "$SINGLE_REPO" ]; then
    echo -e "Testing single repo: ${YELLOW}$SINGLE_REPO${NC}"
    test_repo "$SINGLE_REPO"
else
    # Get all repo keys from config
    REPO_KEYS=$(jq -r 'keys[]' "$CONFIG_FILE")

    for repo_key in $REPO_KEYS; do
        test_repo "$repo_key"
    done
fi

# ========================================
# SUMMARY
# ========================================

echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║                        SUMMARY                             ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  ${GREEN}Passed:${NC}  $TOTAL_PASSED"
echo -e "  ${RED}Failed:${NC}  $TOTAL_FAILED"
echo -e "  ${YELLOW}Skipped:${NC} $TOTAL_SKIPPED"
echo ""

# Save results
RESULTS_FILE="$RESULTS_DIR/test-real-repos-$TIMESTAMP.json"
echo "$RESULTS_JSON" | jq '.' > "$RESULTS_FILE"
echo "Results saved to: $RESULTS_FILE"

# Generate markdown report
REPORT_FILE="$RESULTS_DIR/test-real-repos-$TIMESTAMP.md"
python3 "$SCRIPT_DIR/generate-test-report.py" "$RESULTS_FILE" > "$REPORT_FILE" 2>/dev/null || {
    echo "Note: Run 'python scripts/generate-test-report.py $RESULTS_FILE' to generate markdown report"
}

if [ -f "$REPORT_FILE" ]; then
    echo "Report saved to: $REPORT_FILE"
fi

# Exit code based on failures
if [ "$TOTAL_FAILED" -gt 0 ]; then
    exit 1
fi
exit 0
