#!/bin/bash
# agents_md_vs_mcp.sh - Compare AGENTS.md CLI instructions vs MCP Server
#
# This experiment tests whether well-defined rkt CLI instructions in AGENTS.md
# can achieve comparable tool usage consistency to the MCP server.
#
# Hypothesis: MCP server will show more consistent tool usage because tools
# appear in the agent's tool list, while AGENTS.md requires the agent to
# read and follow markdown instructions.
#
# Usage:
#   ./scripts/benchmarks/agents_md_vs_mcp.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROCKETINDEX_DIR="${SCRIPT_DIR}/../.."
OUTPUT_DIR="${SCRIPT_DIR}/results/agents_md_vs_mcp"
VETS_API_DIR="${HOME}/test-ri/vets-api"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Create output directory
mkdir -p "$OUTPUT_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

echo -e "${BLUE}=== AGENTS.md vs MCP Server Experiment ===${NC}"
echo -e "Timestamp: ${CYAN}$TIMESTAMP${NC}"
echo -e "Output: ${CYAN}$OUTPUT_DIR${NC}"
echo ""

# Test tasks - these should benefit from rkt tools
TASKS=(
    "Find all functions that call ApiProviderFactory. List each caller with file path and function name."
    "Find where the User class is defined in this codebase."
    "What functions call BackendServiceException? List them."
)

# Models to test
MODELS=("haiku" "sonnet")

# Run a single test
run_test() {
    local prompt="$1"
    local model="$2"
    local mode="$3"  # "mcp" or "agents_md"
    local output_file="$4"

    cd "$VETS_API_DIR"

    if [ "$mode" = "mcp" ]; then
        # MCP enabled - agent can use rocketindex tools
        result=$(claude -p "$prompt" \
            --model "$model" \
            --output-format json \
            --max-turns 10 \
            --permission-mode bypassPermissions \
            2>/dev/null)
    else
        # MCP disabled - agent relies on AGENTS.md instructions only
        result=$(claude -p "$prompt" \
            --model "$model" \
            --output-format json \
            --max-turns 10 \
            --disallowedTools "mcp__rocketindex*" \
            2>/dev/null)
    fi

    echo "$result" > "$output_file"
    echo "$result"
}

# Extract key metrics
extract_metrics() {
    local json="$1"
    local turns=$(echo "$json" | jq -r '.num_turns // 0')
    local cost=$(echo "$json" | jq -r '.cost_usd // .total_cost_usd // 0')
    local duration=$(echo "$json" | jq -r '.duration_ms // 0')
    local subtype=$(echo "$json" | jq -r '.subtype // "unknown"')

    # Check if rkt was used (look for rkt or mcp__rocketindex in the result)
    local used_rkt="false"
    if echo "$json" | jq -r '.result // ""' | grep -q "rkt\|find_callers\|find_definition"; then
        used_rkt="true"
    fi

    echo "$turns|$cost|$duration|$subtype|$used_rkt"
}

# Results file
RESULTS_FILE="${OUTPUT_DIR}/results_${TIMESTAMP}.md"

cat > "$RESULTS_FILE" << 'EOF'
# AGENTS.md vs MCP Server Experiment Results

## Hypothesis
MCP server will show more consistent rkt tool usage because tools appear
in the agent's tool list, while AGENTS.md requires the agent to read and
follow markdown instructions.

## Methodology
- Run identical prompts with MCP enabled vs disabled
- MCP disabled forces agent to rely on AGENTS.md instructions
- Measure: turns, cost, and whether agent used rkt commands

## Results

EOF

echo -e "${BLUE}Running tests...${NC}"
echo ""

for model in "${MODELS[@]}"; do
    echo -e "${YELLOW}=== Model: $model ===${NC}"
    echo ""

    echo "### Model: $model" >> "$RESULTS_FILE"
    echo "" >> "$RESULTS_FILE"
    echo "| Task | Mode | Turns | Cost | Used rkt? | Status |" >> "$RESULTS_FILE"
    echo "|------|------|-------|------|-----------|--------|" >> "$RESULTS_FILE"

    task_num=0
    for task in "${TASKS[@]}"; do
        task_num=$((task_num + 1))
        task_short=$(echo "$task" | cut -c1-40)...

        echo -e "${CYAN}Task $task_num: $task_short${NC}"

        # Test with MCP
        echo -n "  MCP enabled:  "
        mcp_file="${OUTPUT_DIR}/${model}_task${task_num}_mcp_${TIMESTAMP}.json"
        mcp_result=$(run_test "$task" "$model" "mcp" "$mcp_file")
        mcp_metrics=$(extract_metrics "$mcp_result")
        IFS='|' read -r mcp_turns mcp_cost mcp_duration mcp_status mcp_used_rkt <<< "$mcp_metrics"
        echo -e "${GREEN}${mcp_turns} turns, \$${mcp_cost}, rkt=${mcp_used_rkt}${NC}"

        # Test without MCP (AGENTS.md only)
        echo -n "  AGENTS.md only: "
        agents_file="${OUTPUT_DIR}/${model}_task${task_num}_agents_${TIMESTAMP}.json"
        agents_result=$(run_test "$task" "$model" "agents_md" "$agents_file")
        agents_metrics=$(extract_metrics "$agents_result")
        IFS='|' read -r agents_turns agents_cost agents_duration agents_status agents_used_rkt <<< "$agents_metrics"
        echo -e "${GREEN}${agents_turns} turns, \$${agents_cost}, rkt=${agents_used_rkt}${NC}"

        # Write to results
        echo "| Task $task_num | MCP | $mcp_turns | \$$mcp_cost | $mcp_used_rkt | $mcp_status |" >> "$RESULTS_FILE"
        echo "| Task $task_num | AGENTS.md | $agents_turns | \$$agents_cost | $agents_used_rkt | $agents_status |" >> "$RESULTS_FILE"

        echo ""
    done

    echo "" >> "$RESULTS_FILE"
done

# Summary
cat >> "$RESULTS_FILE" << 'EOF'

## Analysis

### Key Questions:
1. Did the agent use `rkt` commands when only AGENTS.md was available?
2. Was tool usage more consistent with MCP enabled?
3. Did turn count differ significantly between modes?

### Observations:
(To be filled in after reviewing results)

EOF

echo -e "${BLUE}=== Experiment Complete ===${NC}"
echo -e "Results: ${CYAN}$RESULTS_FILE${NC}"
echo ""
cat "$RESULTS_FILE"
