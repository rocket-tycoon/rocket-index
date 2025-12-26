#!/bin/bash
# Test RocketIndex tools against minimal fixtures
# Usage: ./scripts/test-fixtures.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
RKT="$ROOT_DIR/target/release/rkt"
FIXTURES_DIR="$ROOT_DIR/tests/fixtures/minimal"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Counters
PASSED=0
FAILED=0

# Test helper
assert_json_contains() {
    local json="$1"
    local key="$2"
    local expected="$3"
    local description="$4"

    if echo "$json" | jq -e "$key == \"$expected\"" > /dev/null 2>&1; then
        echo -e "  ${GREEN}✓${NC} $description"
        ((PASSED++))
    else
        echo -e "  ${RED}✗${NC} $description"
        echo "    Expected: $expected"
        echo "    Got: $(echo "$json" | jq "$key")"
        ((FAILED++))
    fi
}

assert_json_array_length() {
    local json="$1"
    local key="$2"
    local expected="$3"
    local description="$4"

    local actual
    actual=$(echo "$json" | jq "$key | length")
    if [ "$actual" -eq "$expected" ]; then
        echo -e "  ${GREEN}✓${NC} $description"
        ((PASSED++))
    else
        echo -e "  ${RED}✗${NC} $description"
        echo "    Expected: $expected items"
        echo "    Got: $actual items"
        ((FAILED++))
    fi
}

assert_json_array_contains() {
    local json="$1"
    local array_key="$2"
    local search_key="$3"
    local search_value="$4"
    local description="$5"

    if echo "$json" | jq -e "$array_key | any(.$search_key == \"$search_value\")" > /dev/null 2>&1; then
        echo -e "  ${GREEN}✓${NC} $description"
        ((PASSED++))
    else
        echo -e "  ${RED}✗${NC} $description"
        echo "    Expected to find $search_key=$search_value in $array_key"
        ((FAILED++))
    fi
}

# Ensure binary exists
if [ ! -x "$RKT" ]; then
    echo -e "${RED}Error: rkt binary not found at $RKT${NC}"
    echo "Run: cargo build --release"
    exit 1
fi

# Ensure fixtures exist
if [ ! -d "$FIXTURES_DIR" ]; then
    echo -e "${YELLOW}Creating minimal fixtures...${NC}"
    "$SCRIPT_DIR/setup-test-fixtures.sh" minimal
fi

echo "================================================"
echo "RocketIndex Fixture Tests"
echo "================================================"
echo ""

# Test each language
for lang in rust python typescript; do
    LANG_DIR="$FIXTURES_DIR/$lang"

    if [ ! -d "$LANG_DIR" ]; then
        echo -e "${YELLOW}⚠ Skipping $lang (not found)${NC}"
        continue
    fi

    echo -e "${YELLOW}Testing $lang fixtures...${NC}"

    # Index the fixture
    echo "  Indexing..."
    INDEX_RESULT=$("$RKT" index --root "$LANG_DIR" --quiet 2>&1)
    SYMBOL_COUNT=$(echo "$INDEX_RESULT" | jq '.symbols')

    if [ "$SYMBOL_COUNT" -gt 0 ]; then
        echo -e "  ${GREEN}✓${NC} Indexed $SYMBOL_COUNT symbols"
        ((PASSED++))
    else
        echo -e "  ${RED}✗${NC} Failed to index"
        ((FAILED++))
        continue
    fi

    # Change to fixture directory for subsequent commands
    cd "$LANG_DIR"

    # Test symbol search
    echo "  Testing symbol search..."
    SYMBOLS=$("$RKT" symbols "*" --quiet 2>&1)
    assert_json_array_length "$SYMBOLS" "." "$SYMBOL_COUNT" "symbols '*' returns all symbols"

    # Language-specific tests
    case $lang in
        rust)
            # Test find_definition
            echo "  Testing find_definition..."
            DEF_RESULT=$("$RKT" def "main_function" --quiet 2>&1)
            assert_json_contains "$DEF_RESULT" ".name" "main_function" "def finds main_function"
            assert_json_contains "$DEF_RESULT" ".kind" "Function" "main_function is a Function"

            # Test find_callers
            echo "  Testing find_callers..."
            CALLERS_RESULT=$("$RKT" callers "main_function" --quiet 2>&1)
            assert_json_array_length "$CALLERS_RESULT" ".callers" 2 "main_function has 2 callers"
            assert_json_array_contains "$CALLERS_RESULT" ".callers" "name" "caller_a" "caller_a calls main_function"
            assert_json_array_contains "$CALLERS_RESULT" ".callers" "name" "caller_b" "caller_b calls main_function"

            # Test module-qualified function
            METHOD_CALLERS=$("$RKT" callers "utils::helper" --quiet 2>&1)
            assert_json_array_length "$METHOD_CALLERS" ".callers" 2 "utils::helper has 2 callers"
            ;;

        python)
            # Test find_definition
            echo "  Testing find_definition..."
            DEF_RESULT=$("$RKT" def "main_function" --quiet 2>&1)
            assert_json_contains "$DEF_RESULT" ".name" "main_function" "def finds main_function"

            # Test find_callers
            echo "  Testing find_callers..."
            CALLERS_RESULT=$("$RKT" callers "main_function" --quiet 2>&1)
            assert_json_array_length "$CALLERS_RESULT" ".callers" 3 "main_function has 3 callers"
            assert_json_array_contains "$CALLERS_RESULT" ".callers" "qualified" "ChildClass.method" "ChildClass.method calls main_function"
            ;;

        typescript)
            # Test find_definition (note: camelCase)
            echo "  Testing find_definition..."
            DEF_RESULT=$("$RKT" def "mainFunction" --quiet 2>&1)
            assert_json_contains "$DEF_RESULT" ".name" "mainFunction" "def finds mainFunction"

            # Test find_callers
            echo "  Testing find_callers..."
            CALLERS_RESULT=$("$RKT" callers "mainFunction" --quiet 2>&1)
            assert_json_array_length "$CALLERS_RESULT" ".callers" 2 "mainFunction has 2 callers"
            assert_json_array_contains "$CALLERS_RESULT" ".callers" "name" "callerA" "callerA calls mainFunction"
            assert_json_array_contains "$CALLERS_RESULT" ".callers" "name" "callerB" "callerB calls mainFunction"
            ;;
    esac

    cd "$ROOT_DIR"
    echo ""
done

# Summary
echo "================================================"
echo "Results: $PASSED passed, $FAILED failed"
echo "================================================"

if [ "$FAILED" -gt 0 ]; then
    exit 1
fi
