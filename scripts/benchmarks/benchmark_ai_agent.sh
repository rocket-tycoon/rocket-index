#!/bin/bash
# benchmark_ai_agent.sh - Measure AI agent efficiency: with vs without RocketIndex
#
# This benchmark runs actual Claude Code sessions to compare real-world performance
# when finding symbol definitions with and without RocketIndex MCP tools.
#
# Metrics captured:
# - duration_ms: Total execution time
# - num_turns: Number of agentic turns (tool call rounds)
# - total_cost_usd: API cost
#
# Usage: ./scripts/benchmark_ai_agent.sh [--runs N] [--repo PATH]
#
# Prerequisites:
# - Claude Code CLI installed
# - rkt binary built: cargo build --release
# - Test repo with .rocketindex indexed

set -e

# Defaults
RUNS=3
REPO_PATH=""
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RKT="${SCRIPT_DIR}/../target/release/rkt"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --runs) RUNS="$2"; shift 2 ;;
        --repo) REPO_PATH="$2"; shift 2 ;;
        *) shift ;;
    esac
done

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Find a test repo if not specified
if [ -z "$REPO_PATH" ]; then
    if [ -d "${SCRIPT_DIR}/../test-repos/rust/tokio" ]; then
        REPO_PATH="${SCRIPT_DIR}/../test-repos/rust/tokio"
    else
        echo "Error: No repo specified and test-repos/rust/tokio not found" >&2
        exit 1
    fi
fi

cd "$REPO_PATH"
REPO_NAME=$(basename "$REPO_PATH")

echo -e "${BLUE}=== AI Agent Benchmark: RocketIndex vs Standard Tools ===${NC}"
echo -e "Repo: ${YELLOW}$REPO_NAME${NC} ($(pwd))"
echo -e "Runs: ${CYAN}$RUNS${NC} per scenario"
echo ""

# Ensure index exists
if [ ! -f ".rocketindex/index.db" ]; then
    echo -e "${YELLOW}Building RocketIndex...${NC}"
    "$RKT" index --quiet
fi

# Get repo stats
FILE_COUNT=$(find . -name "*.rs" 2>/dev/null | wc -l | tr -d ' ')
SYMBOL_COUNT=$(sqlite3 .rocketindex/index.db "SELECT COUNT(*) FROM symbols;" 2>/dev/null || echo "0")
echo -e "Files: ${GREEN}$FILE_COUNT${NC}, Symbols: ${GREEN}$SYMBOL_COUNT${NC}"
echo ""

# The benchmark task - find a symbol definition
SYMBOL="spawn"
TASK="Find where the function '$SYMBOL' is defined in this codebase. Return just the file path and line number."

# Run benchmark without RocketIndex
run_without_rkt() {
    local output
    output=$(claude -p "$TASK" \
        --output-format json \
        --disallowedTools "mcp__rocketindex*" \
        --max-turns 10 \
        2>/dev/null)
    echo "$output"
}

# Run benchmark with RocketIndex
run_with_rkt() {
    local output
    output=$(claude -p "$TASK" \
        --output-format json \
        --max-turns 10 \
        2>/dev/null)
    echo "$output"
}

# Extract metrics from JSON output
extract_metrics() {
    local json="$1"
    local duration=$(echo "$json" | jq -r '.duration_ms // 0')
    local turns=$(echo "$json" | jq -r '.num_turns // 0')
    local cost=$(echo "$json" | jq -r '.total_cost_usd // 0')
    echo "$duration $turns $cost"
}

echo -e "${BLUE}--- Benchmark: Find '$SYMBOL' definition ---${NC}"
echo ""

# Arrays to store results
declare -a WITHOUT_DURATIONS WITHOUT_TURNS WITHOUT_COSTS
declare -a WITH_DURATIONS WITH_TURNS WITH_COSTS

# Run without RocketIndex
echo -e "${YELLOW}Running WITHOUT RocketIndex ($RUNS runs)...${NC}"
for i in $(seq 1 $RUNS); do
    echo -n "  Run $i: "
    result=$(run_without_rkt)
    metrics=$(extract_metrics "$result")
    read duration turns cost <<< "$metrics"
    WITHOUT_DURATIONS+=($duration)
    WITHOUT_TURNS+=($turns)
    WITHOUT_COSTS+=($cost)
    echo "${duration}ms, ${turns} turns, \$${cost}"
done
echo ""

# Run with RocketIndex
echo -e "${GREEN}Running WITH RocketIndex ($RUNS runs)...${NC}"
for i in $(seq 1 $RUNS); do
    echo -n "  Run $i: "
    result=$(run_with_rkt)
    metrics=$(extract_metrics "$result")
    read duration turns cost <<< "$metrics"
    WITH_DURATIONS+=($duration)
    WITH_TURNS+=($turns)
    WITH_COSTS+=($cost)
    echo "${duration}ms, ${turns} turns, \$${cost}"
done
echo ""

# Calculate averages
avg() {
    local arr=("$@")
    local sum=0
    for val in "${arr[@]}"; do
        sum=$(echo "$sum + $val" | bc)
    done
    echo "scale=2; $sum / ${#arr[@]}" | bc
}

WITHOUT_AVG_DURATION=$(avg "${WITHOUT_DURATIONS[@]}")
WITHOUT_AVG_TURNS=$(avg "${WITHOUT_TURNS[@]}")
WITHOUT_AVG_COST=$(avg "${WITHOUT_COSTS[@]}")

WITH_AVG_DURATION=$(avg "${WITH_DURATIONS[@]}")
WITH_AVG_TURNS=$(avg "${WITH_TURNS[@]}")
WITH_AVG_COST=$(avg "${WITH_COSTS[@]}")

# Calculate improvements
DURATION_IMPROVEMENT=$(echo "scale=0; ($WITHOUT_AVG_DURATION - $WITH_AVG_DURATION) * 100 / $WITHOUT_AVG_DURATION" | bc 2>/dev/null || echo "0")
TURNS_IMPROVEMENT=$(echo "scale=1; $WITHOUT_AVG_TURNS - $WITH_AVG_TURNS" | bc)
COST_IMPROVEMENT=$(echo "scale=0; ($WITHOUT_AVG_COST - $WITH_AVG_COST) * 100 / $WITHOUT_AVG_COST" | bc 2>/dev/null || echo "0")

# Print results
echo -e "${BLUE}=== Results ===${NC}"
echo ""
echo "| Metric          | Without RKT | With RKT    | Improvement |"
echo "|-----------------|-------------|-------------|-------------|"
printf "| Duration (ms)   | %11.0f | %11.0f | %10s%% |\n" "$WITHOUT_AVG_DURATION" "$WITH_AVG_DURATION" "$DURATION_IMPROVEMENT"
printf "| Tool turns      | %11.1f | %11.1f | %+10.1f |\n" "$WITHOUT_AVG_TURNS" "$WITH_AVG_TURNS" "$TURNS_IMPROVEMENT"
printf "| Cost (USD)      | %11.4f | %11.4f | %10s%% |\n" "$WITHOUT_AVG_COST" "$WITH_AVG_COST" "$COST_IMPROVEMENT"
echo ""

# JSON output for CI/automation
echo -e "${CYAN}JSON output:${NC}"
cat <<EOF
{
  "benchmark": "ai_agent_symbol_lookup",
  "repo": "$REPO_NAME",
  "symbol": "$SYMBOL",
  "runs": $RUNS,
  "without_rocketindex": {
    "avg_duration_ms": $WITHOUT_AVG_DURATION,
    "avg_turns": $WITHOUT_AVG_TURNS,
    "avg_cost_usd": $WITHOUT_AVG_COST
  },
  "with_rocketindex": {
    "avg_duration_ms": $WITH_AVG_DURATION,
    "avg_turns": $WITH_AVG_TURNS,
    "avg_cost_usd": $WITH_AVG_COST
  },
  "improvement": {
    "duration_percent": $DURATION_IMPROVEMENT,
    "turns_saved": $TURNS_IMPROVEMENT,
    "cost_percent": $COST_IMPROVEMENT
  }
}
EOF
