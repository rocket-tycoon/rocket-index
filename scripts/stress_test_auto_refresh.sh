#!/usr/bin/env bash
# Stress test: Auto-refresh with large file counts
# Tests that auto-refresh scales to large projects

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TEST_DIR="/tmp/rkt_stress_test"
RKT="$PROJECT_ROOT/target/release/rkt"

# Test parameters
FILE_COUNTS="100 500 1000 2000"
MODIFIED_COUNTS="1 10 50 100"

echo "=== Auto-Refresh Stress Test ==="
echo "Testing auto-refresh scalability with large file counts"
echo ""

# Ensure we have a release build
echo "Building release..."
cd "$PROJECT_ROOT"
cargo build --release --quiet

# Function to create test files
create_test_files() {
    local count=$1
    local dir=$2
    rm -rf "$dir"
    mkdir -p "$dir"

    echo -n "Creating $count files..."
    for i in $(seq 1 $count); do
        cat > "$dir/file_$i.rs" << EOF
pub struct Struct$i { field: i32 }
impl Struct$i {
    pub fn new() -> Self { Self { field: $i } }
}
pub fn helper_$i() -> i32 { $i }
EOF
    done
    echo " done"
}

# Function to measure staleness check time (no files modified)
measure_staleness_check() {
    local dir=$1
    cd "$dir"

    start=$(python3 -c 'import time; print(time.time())')
    "$RKT" def "Struct1" --quiet > /dev/null 2>&1
    end=$(python3 -c 'import time; print(time.time())')

    python3 -c "print(f'{($end - $start)*1000:.1f}')"
}

# Function to measure refresh time
measure_refresh() {
    local dir=$1
    local modify_count=$2

    cd "$dir"

    # Modify specified number of files
    for i in $(seq 1 $modify_count); do
        echo "// modified $(date +%s%N)" >> "file_$i.rs"
    done

    start=$(python3 -c 'import time; print(time.time())')
    "$RKT" def "Struct1" --quiet > /dev/null 2>&1
    end=$(python3 -c 'import time; print(time.time())')

    python3 -c "print(f'{($end - $start)*1000:.1f}')"
}

echo ""
echo "=== Test 1: Staleness check overhead (no files modified) ==="
printf "%-12s %12s\n" "Total Files" "Check (ms)"
printf "%-12s %12s\n" "-----------" "----------"

for total in $FILE_COUNTS; do
    create_test_files $total "$TEST_DIR"
    cd "$TEST_DIR"
    "$RKT" index --quiet > /dev/null 2>&1

    # Warm up
    "$RKT" def "Struct1" --quiet > /dev/null 2>&1

    # Measure 3 times and average
    times=""
    for _ in 1 2 3; do
        ms=$(measure_staleness_check "$TEST_DIR")
        times="$times $ms"
    done
    avg=$(echo $times | tr ' ' '\n' | grep -v '^$' | python3 -c "import sys; nums=[float(x) for x in sys.stdin.read().split()]; print(f'{sum(nums)/len(nums):.1f}')")
    printf "%-12s %12s\n" "$total" "$avg"
done

echo ""
echo "=== Test 2: Refresh latency (varying files modified) ==="
echo "Total files: 1000"
echo ""
printf "%-12s %12s\n" "Modified" "Refresh (ms)"
printf "%-12s %12s\n" "--------" "------------"

create_test_files 1000 "$TEST_DIR"
cd "$TEST_DIR"

for modified in $MODIFIED_COUNTS; do
    # Fresh index each time
    "$RKT" index --quiet > /dev/null 2>&1

    ms=$(measure_refresh "$TEST_DIR" $modified)
    printf "%-12s %12s\n" "$modified" "$ms"
done

echo ""
echo "=== Test 3: Large project stress test (2000 files, 100 modified) ==="

create_test_files 2000 "$TEST_DIR"
cd "$TEST_DIR"
"$RKT" index --quiet > /dev/null 2>&1

echo "Modifying 100 files and refreshing..."
times=""
for run in 1 2 3; do
    # Re-index to reset
    "$RKT" index --quiet > /dev/null 2>&1
    ms=$(measure_refresh "$TEST_DIR" 100)
    times="$times $ms"
    echo "  Run $run: ${ms}ms"
done

avg=$(echo $times | tr ' ' '\n' | grep -v '^$' | python3 -c "import sys; nums=[float(x) for x in sys.stdin.read().split()]; print(f'{sum(nums)/len(nums):.1f}')")
echo "  Average: ${avg}ms"

# Pass/Fail criteria
threshold=500
if python3 -c "exit(0 if $avg < $threshold else 1)"; then
    echo ""
    echo "✅ PASS: Average refresh time (${avg}ms) is under ${threshold}ms threshold"
    result=0
else
    echo ""
    echo "❌ FAIL: Average refresh time (${avg}ms) exceeds ${threshold}ms threshold"
    result=1
fi

# Cleanup
rm -rf "$TEST_DIR"

echo ""
echo "Stress test complete."
exit $result
