#!/bin/bash
# run_benchmark.sh - Run formalized AI agent benchmarks from task definition files
#
# This script runs Claude Code sessions against task definitions, comparing
# performance with and without RocketIndex MCP tools across multiple models.
#
# Usage:
#   ./scripts/benchmarks/run_benchmark.sh --task-file tasks/ruby_vets_api.json --model sonnet
#   ./scripts/benchmarks/run_benchmark.sh --task-file tasks/rust_tokio.json --model haiku --runs 3
#   ./scripts/benchmarks/run_benchmark.sh --task-file tasks/ruby_vets_api.json --task-id find_callers_factory
#
# Options:
#   --task-file FILE    JSON file with task definitions (required)
#   --model MODEL       sonnet or haiku (default: sonnet)
#   --runs N            Number of runs per task (default: 1)
#   --task-id ID        Run only a specific task (optional)
#   --ensure-indexed    Verify index exists before running
#   --output-dir DIR    Directory for results (default: results/)
#   --verbose           Show detailed output

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROCKETINDEX_DIR="${SCRIPT_DIR}/../.."
RKT="${ROCKETINDEX_DIR}/target/release/rkt"

# Defaults
TASK_FILE=""
MODEL="sonnet"
RUNS=1
TASK_ID=""
ENSURE_INDEXED=false
OUTPUT_DIR="${SCRIPT_DIR}/results"
VERBOSE=false
MAX_TURNS=15

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --task-file) TASK_FILE="$2"; shift 2 ;;
        --model) MODEL="$2"; shift 2 ;;
        --runs) RUNS="$2"; shift 2 ;;
        --task-id) TASK_ID="$2"; shift 2 ;;
        --ensure-indexed) ENSURE_INDEXED=true; shift ;;
        --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
        --verbose) VERBOSE=true; shift ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Validate inputs
if [ -z "$TASK_FILE" ]; then
    echo -e "${RED}Error: --task-file is required${NC}" >&2
    exit 1
fi

if [ ! -f "$TASK_FILE" ] && [ ! -f "${SCRIPT_DIR}/$TASK_FILE" ]; then
    echo -e "${RED}Error: Task file not found: $TASK_FILE${NC}" >&2
    exit 1
fi

# Resolve task file path
if [ -f "$TASK_FILE" ]; then
    TASK_FILE_PATH="$TASK_FILE"
else
    TASK_FILE_PATH="${SCRIPT_DIR}/$TASK_FILE"
fi

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Cross-platform millisecond timestamp
get_ms() {
    if [[ "$OSTYPE" == "darwin"* ]]; then
        perl -MTime::HiRes=time -e 'printf "%.0f\n", time * 1000'
    else
        date +%s%3N
    fi
}

# Read task file
REPO=$(jq -r '.repo' "$TASK_FILE_PATH")
LANGUAGE=$(jq -r '.language' "$TASK_FILE_PATH")
DESCRIPTION=$(jq -r '.description' "$TASK_FILE_PATH")
TASKS=$(jq -c '.tasks[]' "$TASK_FILE_PATH")

# Expand ~ in repo path
REPO="${REPO/#\~/$HOME}"

echo -e "${BLUE}=== RocketIndex Benchmark Suite ===${NC}"
echo -e "Task file: ${CYAN}$(basename "$TASK_FILE_PATH")${NC}"
echo -e "Language:  ${CYAN}$LANGUAGE${NC}"
echo -e "Repo:      ${YELLOW}$REPO${NC}"
echo -e "Model:     ${GREEN}$MODEL${NC}"
echo -e "Runs:      ${CYAN}$RUNS${NC} per scenario"
echo ""

# Change to repo directory
cd "$REPO" || { echo -e "${RED}Error: Cannot cd to $REPO${NC}"; exit 1; }

# Verify index if requested
if [ "$ENSURE_INDEXED" = true ]; then
    if [ ! -f ".rocketindex/index.db" ]; then
        echo -e "${YELLOW}Index not found. Building...${NC}"
        "$RKT" index --quiet
    else
        INDEX_AGE=$(( $(date +%s) - $(stat -f %m .rocketindex/index.db 2>/dev/null || stat -c %Y .rocketindex/index.db 2>/dev/null) ))
        echo -e "${GREEN}Using existing index (age: ${INDEX_AGE}s)${NC}"
    fi
fi

# Get repo stats
SYMBOL_COUNT=$(sqlite3 .rocketindex/index.db "SELECT COUNT(*) FROM symbols;" 2>/dev/null || echo "0")
REF_COUNT=$(sqlite3 .rocketindex/index.db "SELECT COUNT(*) FROM refs;" 2>/dev/null || echo "0")
echo -e "Symbols: ${GREEN}$SYMBOL_COUNT${NC}, References: ${GREEN}$REF_COUNT${NC}"
echo ""

# Run a single benchmark
run_benchmark() {
    local prompt="$1"
    local with_rkt="$2"

    local output
    if [ "$with_rkt" = "true" ]; then
        # Allow MCP tools without interactive permission prompts
        output=$(claude -p "$prompt" \
            --model "$MODEL" \
            --output-format json \
            --max-turns "$MAX_TURNS" \
            --allowedTools "mcp__rocketindex*" \
            2>/dev/null)
    else
        output=$(claude -p "$prompt" \
            --model "$MODEL" \
            --output-format json \
            --disallowedTools "mcp__rocketindex*" \
            --max-turns "$MAX_TURNS" \
            2>/dev/null)
    fi
    echo "$output"
}

# Extract metrics from JSON
extract_metrics() {
    local json="$1"
    local duration=$(echo "$json" | jq -r '.duration_ms // 0')
    local turns=$(echo "$json" | jq -r '.num_turns // 0')
    local cost=$(echo "$json" | jq -r '.total_cost_usd // 0')
    local result_text=$(echo "$json" | jq -r '.result // ""')
    local subtype=$(echo "$json" | jq -r '.subtype // "unknown"')
    echo "$duration|$turns|$cost|$subtype"
}

# Results array
RESULTS_JSON="[]"

# Process each task
echo "$TASKS" | while IFS= read -r task; do
    TASK_ID_CURRENT=$(echo "$task" | jq -r '.id')

    # Skip if specific task requested and this isn't it
    if [ -n "$TASK_ID" ] && [ "$TASK_ID" != "$TASK_ID_CURRENT" ]; then
        continue
    fi

    CATEGORY=$(echo "$task" | jq -r '.category')
    DIFFICULTY=$(echo "$task" | jq -r '.difficulty')
    PROMPT=$(echo "$task" | jq -r '.prompt')

    echo -e "${BLUE}--- Task: $TASK_ID_CURRENT ($CATEGORY, $DIFFICULTY) ---${NC}"
    echo -e "Prompt: ${CYAN}$PROMPT${NC}"
    echo ""

    # Arrays for this task's results
    WITHOUT_RESULTS=()
    WITH_RESULTS=()

    # Run WITHOUT RocketIndex
    echo -e "${YELLOW}WITHOUT RocketIndex:${NC}"
    for i in $(seq 1 $RUNS); do
        echo -n "  Run $i: "
        result=$(run_benchmark "$PROMPT" "false")
        metrics=$(extract_metrics "$result")
        IFS='|' read -r duration turns cost subtype <<< "$metrics"
        WITHOUT_RESULTS+=("$duration|$turns|$cost|$subtype")

        if [ "$subtype" = "success" ]; then
            echo -e "${GREEN}${turns} turns${NC}, ${duration}ms, \$${cost}"
        else
            echo -e "${RED}${subtype}${NC} after ${turns} turns, ${duration}ms"
        fi

        # Save raw output if verbose
        if [ "$VERBOSE" = true ]; then
            echo "$result" > "${OUTPUT_DIR}/${TASK_ID_CURRENT}_${MODEL}_without_run${i}.json"
        fi
    done

    # Run WITH RocketIndex
    echo -e "${GREEN}WITH RocketIndex:${NC}"
    for i in $(seq 1 $RUNS); do
        echo -n "  Run $i: "
        result=$(run_benchmark "$PROMPT" "true")
        metrics=$(extract_metrics "$result")
        IFS='|' read -r duration turns cost subtype <<< "$metrics"
        WITH_RESULTS+=("$duration|$turns|$cost|$subtype")

        if [ "$subtype" = "success" ]; then
            echo -e "${GREEN}${turns} turns${NC}, ${duration}ms, \$${cost}"
        else
            echo -e "${RED}${subtype}${NC} after ${turns} turns, ${duration}ms"
        fi

        # Save raw output if verbose
        if [ "$VERBOSE" = true ]; then
            echo "$result" > "${OUTPUT_DIR}/${TASK_ID_CURRENT}_${MODEL}_with_run${i}.json"
        fi
    done

    # Calculate averages for this task
    WITHOUT_TURNS_SUM=0
    WITHOUT_SUCCESS=0
    WITH_TURNS_SUM=0
    WITH_SUCCESS=0

    for r in "${WITHOUT_RESULTS[@]}"; do
        IFS='|' read -r d t c s <<< "$r"
        WITHOUT_TURNS_SUM=$((WITHOUT_TURNS_SUM + t))
        [ "$s" = "success" ] && WITHOUT_SUCCESS=$((WITHOUT_SUCCESS + 1))
    done

    for r in "${WITH_RESULTS[@]}"; do
        IFS='|' read -r d t c s <<< "$r"
        WITH_TURNS_SUM=$((WITH_TURNS_SUM + t))
        [ "$s" = "success" ] && WITH_SUCCESS=$((WITH_SUCCESS + 1))
    done

    WITHOUT_AVG_TURNS=$(echo "scale=1; $WITHOUT_TURNS_SUM / $RUNS" | bc)
    WITH_AVG_TURNS=$(echo "scale=1; $WITH_TURNS_SUM / $RUNS" | bc)

    # Calculate turn reduction percentage
    if [ "$WITHOUT_AVG_TURNS" != "0" ]; then
        TURN_REDUCTION=$(echo "scale=0; ($WITHOUT_AVG_TURNS - $WITH_AVG_TURNS) * 100 / $WITHOUT_AVG_TURNS" | bc 2>/dev/null || echo "0")
    else
        TURN_REDUCTION="N/A"
    fi

    echo ""
    echo -e "Summary: Without=${WITHOUT_AVG_TURNS} turns (${WITHOUT_SUCCESS}/${RUNS} success), With=${WITH_AVG_TURNS} turns (${WITH_SUCCESS}/${RUNS} success), ${TURN_REDUCTION}% turn reduction"
    echo ""

    # Build result JSON for this task
    TASK_RESULT=$(cat <<EOF
{
  "task_id": "$TASK_ID_CURRENT",
  "category": "$CATEGORY",
  "difficulty": "$DIFFICULTY",
  "model": "$MODEL",
  "runs": $RUNS,
  "without_rkt": {
    "avg_turns": $WITHOUT_AVG_TURNS,
    "success_rate": $(echo "scale=2; $WITHOUT_SUCCESS / $RUNS" | bc)
  },
  "with_rkt": {
    "avg_turns": $WITH_AVG_TURNS,
    "success_rate": $(echo "scale=2; $WITH_SUCCESS / $RUNS" | bc)
  },
  "turn_reduction_percent": $TURN_REDUCTION
}
EOF
)

    # Append to results file
    TIMESTAMP=$(date +%Y%m%d_%H%M%S)
    RESULT_FILE="${OUTPUT_DIR}/${LANGUAGE}_${MODEL}_${TASK_ID_CURRENT}_${TIMESTAMP}.json"
    echo "$TASK_RESULT" > "$RESULT_FILE"
    echo -e "Result saved: ${CYAN}$RESULT_FILE${NC}"
    echo ""
done

echo -e "${BLUE}=== Benchmark Complete ===${NC}"
echo -e "Results in: ${CYAN}$OUTPUT_DIR${NC}"
