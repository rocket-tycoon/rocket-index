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

assert_json_array_contains_file() {
    local json="$1"
    local array_key="$2"
    local name_value="$3"
    local file_pattern="$4"
    local description="$5"

    if echo "$json" | jq -e "$array_key | any(.name == \"$name_value\" and (.file | contains(\"$file_pattern\")))" > /dev/null 2>&1; then
        echo -e "  ${GREEN}✓${NC} $description"
        ((PASSED++))
    else
        echo -e "  ${RED}✗${NC} $description"
        echo "    Expected to find name=$name_value in file matching $file_pattern"
        ((FAILED++))
    fi
}

assert_json_line() {
    local json="$1"
    local expected_line="$2"
    local description="$3"

    local actual
    actual=$(echo "$json" | jq '.line')
    if [ "$actual" -eq "$expected_line" ]; then
        echo -e "  ${GREEN}✓${NC} $description"
        ((PASSED++))
    else
        echo -e "  ${RED}✗${NC} $description"
        echo "    Expected line: $expected_line, got: $actual"
        ((FAILED++))
    fi
}

assert_def_has_fields() {
    local json="$1"
    local description="$2"

    if echo "$json" | jq -e 'has("name") and has("qualified") and has("file") and has("line") and has("column") and has("kind")' > /dev/null 2>&1; then
        echo -e "  ${GREEN}✓${NC} $description"
        ((PASSED++))
    else
        echo -e "  ${RED}✗${NC} $description"
        echo "    Missing required fields in definition"
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
            # Test find_definition - basic
            echo "  Testing find_definition..."
            DEF_RESULT=$("$RKT" def "main_function" --quiet 2>&1)
            assert_json_contains "$DEF_RESULT" ".name" "main_function" "def finds main_function"
            assert_json_contains "$DEF_RESULT" ".kind" "Function" "main_function is a Function"
            assert_json_line "$DEF_RESULT" 11 "main_function is on line 11"
            assert_def_has_fields "$DEF_RESULT" "definition has required fields"

            # Test qualified name disambiguation
            echo "  Testing qualified name disambiguation..."
            MYSTRUCT_NEW=$("$RKT" def "MyStruct::new" --quiet 2>&1)
            assert_json_contains "$MYSTRUCT_NEW" ".qualified" "MyStruct::new" "qualified name is MyStruct::new"
            assert_json_line "$MYSTRUCT_NEW" 30 "MyStruct::new is on line 30"

            OTHERSTRUCT_NEW=$("$RKT" def "OtherStruct::new" --quiet 2>&1)
            assert_json_contains "$OTHERSTRUCT_NEW" ".qualified" "OtherStruct::new" "qualified name is OtherStruct::new"
            assert_json_line "$OTHERSTRUCT_NEW" 56 "OtherStruct::new is on line 56"

            # Test common names with disambiguation
            echo "  Testing common names..."
            INIT_RESULT=$("$RKT" def "OtherStruct::init" --quiet 2>&1)
            assert_json_contains "$INIT_RESULT" ".name" "init" "found init method"
            assert_json_contains "$INIT_RESULT" ".qualified" "OtherStruct::init" "init is qualified"

            RUN_RESULT=$("$RKT" def "OtherStruct::run" --quiet 2>&1)
            assert_json_contains "$RUN_RESULT" ".name" "run" "found run method"
            assert_json_contains "$RUN_RESULT" ".qualified" "OtherStruct::run" "run is qualified"

            # Test find_callers (same file)
            echo "  Testing find_callers (same file)..."
            CALLERS_RESULT=$("$RKT" callers "main_function" --quiet 2>&1)
            assert_json_array_contains "$CALLERS_RESULT" ".callers" "name" "caller_a" "caller_a calls main_function"
            assert_json_array_contains "$CALLERS_RESULT" ".callers" "name" "caller_b" "caller_b calls main_function"

            # Test cross-file caller detection
            echo "  Testing cross-file caller detection..."
            assert_json_array_contains "$CALLERS_RESULT" ".callers" "name" "cross_file_caller" "cross_file_caller from caller.rs calls main_function"
            assert_json_array_contains_file "$CALLERS_RESULT" ".callers" "cross_file_caller" "caller.rs" "cross_file_caller is in caller.rs"

            # Test module-qualified function callers
            echo "  Testing qualified name callers..."
            HELPER_CALLERS=$("$RKT" callers "utils::helper" --quiet 2>&1)
            assert_json_array_contains "$HELPER_CALLERS" ".callers" "name" "main_function" "main_function calls utils::helper"
            assert_json_array_contains "$HELPER_CALLERS" ".callers" "name" "cross_file_caller" "cross_file_caller calls utils::helper"

            # Test JSON output format
            echo "  Testing JSON output format..."
            FIRST_CALLER=$(echo "$CALLERS_RESULT" | jq '.callers[0]')
            if echo "$FIRST_CALLER" | jq -e 'has("name") and has("file") and has("line") and has("column") and has("kind")' > /dev/null 2>&1; then
                echo -e "  ${GREEN}✓${NC} caller has required fields (name, file, line, column, kind)"
                ((PASSED++))
            else
                echo -e "  ${RED}✗${NC} caller missing required fields"
                ((FAILED++))
            fi

            # Test analyze_dependencies (spider) - forward traversal
            echo "  Testing forward dependency traversal..."
            SPIDER_FWD=$("$RKT" spider "main_function" --depth 2 --quiet 2>&1)
            assert_json_array_contains "$SPIDER_FWD" ".nodes" "qualified" "main_function" "spider includes entry point"
            assert_json_array_contains "$SPIDER_FWD" ".nodes" "qualified" "utils::helper" "spider finds utils::helper dependency"

            # Test reverse traversal (impact analysis)
            echo "  Testing reverse dependency traversal..."
            SPIDER_REV=$("$RKT" spider "main_function" --reverse --depth 2 --quiet 2>&1)
            assert_json_array_contains "$SPIDER_REV" ".nodes" "name" "caller_a" "reverse spider finds caller_a"
            assert_json_array_contains "$SPIDER_REV" ".nodes" "name" "cross_file_caller" "reverse spider finds cross_file_caller"
            assert_json_array_contains "$SPIDER_REV" ".nodes" "name" "another_caller" "reverse spider finds another_caller at depth 2"

            # Test depth limiting
            echo "  Testing depth limiting..."
            SPIDER_D1=$("$RKT" spider "main_function" --reverse --depth 1 --quiet 2>&1)
            DEPTH1_COUNT=$(echo "$SPIDER_D1" | jq '[.nodes[] | select(.depth <= 1)] | length')
            TOTAL_COUNT=$(echo "$SPIDER_D1" | jq '.nodes | length')
            if [ "$DEPTH1_COUNT" -eq "$TOTAL_COUNT" ]; then
                echo -e "  ${GREEN}✓${NC} depth=1 limits to depth 0 and 1 only"
                ((PASSED++))
            else
                echo -e "  ${RED}✗${NC} depth limiting not working"
                ((FAILED++))
            fi

            # Test cycle detection (mutual recursion)
            echo "  Testing cycle detection..."
            SPIDER_CYCLE=$("$RKT" spider "cycle_a" --depth 5 --quiet 2>&1)
            # Should find cycle_b but not recurse infinitely
            assert_json_array_contains "$SPIDER_CYCLE" ".nodes" "name" "cycle_a" "cycle starts at cycle_a"
            assert_json_array_contains "$SPIDER_CYCLE" ".nodes" "name" "cycle_b" "cycle includes cycle_b"
            CYCLE_COUNT=$(echo "$SPIDER_CYCLE" | jq '.nodes | length')
            if [ "$CYCLE_COUNT" -lt 10 ]; then
                echo -e "  ${GREEN}✓${NC} cycle detection prevents infinite recursion ($CYCLE_COUNT nodes)"
                ((PASSED++))
            else
                echo -e "  ${RED}✗${NC} possible infinite recursion ($CYCLE_COUNT nodes)"
                ((FAILED++))
            fi
            ;;

        python)
            # Test find_definition - basic
            echo "  Testing find_definition..."
            DEF_RESULT=$("$RKT" def "main_function" --quiet 2>&1)
            assert_json_contains "$DEF_RESULT" ".name" "main_function" "def finds main_function"
            assert_json_line "$DEF_RESULT" 7 "main_function is on line 7"
            assert_def_has_fields "$DEF_RESULT" "definition has required fields"

            # Test qualified name disambiguation
            echo "  Testing qualified name disambiguation..."
            MYCLASS_INIT=$("$RKT" def "MyClass.__init__" --quiet 2>&1)
            assert_json_contains "$MYCLASS_INIT" ".qualified" "MyClass.__init__" "qualified name is MyClass.__init__"

            OTHERCLASS_INIT=$("$RKT" def "OtherClass.init" --quiet 2>&1)
            assert_json_contains "$OTHERCLASS_INIT" ".qualified" "OtherClass.init" "qualified name is OtherClass.init"
            assert_json_line "$OTHERCLASS_INIT" 44 "OtherClass.init is on line 44"

            # Test common names
            echo "  Testing common names..."
            RUN_RESULT=$("$RKT" def "OtherClass.run" --quiet 2>&1)
            assert_json_contains "$RUN_RESULT" ".name" "run" "found run method"
            assert_json_contains "$RUN_RESULT" ".qualified" "OtherClass.run" "run is qualified"

            # Test find_callers (same file)
            echo "  Testing find_callers (same file)..."
            CALLERS_RESULT=$("$RKT" callers "main_function" --quiet 2>&1)
            assert_json_array_contains "$CALLERS_RESULT" ".callers" "qualified" "ChildClass.method" "ChildClass.method calls main_function"

            # Test cross-file caller detection
            echo "  Testing cross-file caller detection..."
            assert_json_array_contains "$CALLERS_RESULT" ".callers" "name" "cross_file_caller" "cross_file_caller from caller.py calls main_function"
            assert_json_array_contains_file "$CALLERS_RESULT" ".callers" "cross_file_caller" "caller.py" "cross_file_caller is in caller.py"

            # Test JSON output format
            echo "  Testing JSON output format..."
            FIRST_CALLER=$(echo "$CALLERS_RESULT" | jq '.callers[0]')
            if echo "$FIRST_CALLER" | jq -e 'has("name") and has("file") and has("line") and has("column") and has("kind")' > /dev/null 2>&1; then
                echo -e "  ${GREEN}✓${NC} caller has required fields"
                ((PASSED++))
            else
                echo -e "  ${RED}✗${NC} caller missing required fields"
                ((FAILED++))
            fi
            ;;

        typescript)
            # Test find_definition - basic
            echo "  Testing find_definition..."
            DEF_RESULT=$("$RKT" def "mainFunction" --quiet 2>&1)
            assert_json_contains "$DEF_RESULT" ".name" "mainFunction" "def finds mainFunction"
            assert_json_line "$DEF_RESULT" 7 "mainFunction is on line 7"
            assert_def_has_fields "$DEF_RESULT" "definition has required fields"

            # Test qualified name disambiguation
            echo "  Testing qualified name disambiguation..."
            MYCLASS_METHOD=$("$RKT" def "MyClass.method" --quiet 2>&1)
            assert_json_contains "$MYCLASS_METHOD" ".qualified" "MyClass.method" "qualified name is MyClass.method"

            OTHERCLASS_INIT=$("$RKT" def "OtherClass.init" --quiet 2>&1)
            assert_json_contains "$OTHERCLASS_INIT" ".qualified" "OtherClass.init" "qualified name is OtherClass.init"
            assert_json_line "$OTHERCLASS_INIT" 51 "OtherClass.init is on line 51"

            # Test common names
            echo "  Testing common names..."
            RUN_RESULT=$("$RKT" def "OtherClass.run" --quiet 2>&1)
            assert_json_contains "$RUN_RESULT" ".name" "run" "found run method"
            assert_json_contains "$RUN_RESULT" ".qualified" "OtherClass.run" "run is qualified"

            # Test find_callers (same file)
            echo "  Testing find_callers (same file)..."
            CALLERS_RESULT=$("$RKT" callers "mainFunction" --quiet 2>&1)
            assert_json_array_contains "$CALLERS_RESULT" ".callers" "name" "callerA" "callerA calls mainFunction"
            assert_json_array_contains "$CALLERS_RESULT" ".callers" "name" "callerB" "callerB calls mainFunction"

            # Test cross-file caller detection
            echo "  Testing cross-file caller detection..."
            assert_json_array_contains "$CALLERS_RESULT" ".callers" "name" "crossFileCaller" "crossFileCaller from caller.ts calls mainFunction"
            assert_json_array_contains_file "$CALLERS_RESULT" ".callers" "crossFileCaller" "caller.ts" "crossFileCaller is in caller.ts"

            # Test JSON output format
            echo "  Testing JSON output format..."
            FIRST_CALLER=$(echo "$CALLERS_RESULT" | jq '.callers[0]')
            if echo "$FIRST_CALLER" | jq -e 'has("name") and has("file") and has("line") and has("column") and has("kind")' > /dev/null 2>&1; then
                echo -e "  ${GREEN}✓${NC} caller has required fields"
                ((PASSED++))
            else
                echo -e "  ${RED}✗${NC} caller missing required fields"
                ((FAILED++))
            fi
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
