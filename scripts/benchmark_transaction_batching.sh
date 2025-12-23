#!/usr/bin/env bash
# Benchmark: Transaction batching A/B comparison
# Uses git worktree to safely build old version without modifying working directory

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TEST_DIR="/tmp/rkt_txn_benchmark"
WORKTREE_DIR="/tmp/rkt_old_worktree"
RKT_NEW="$PROJECT_ROOT/target/release/rkt"
RKT_OLD="$WORKTREE_DIR/target/release/rkt"

# Test parameters
FILE_COUNT=100
RUNS=5

# Cleanup function
cleanup() {
    echo "Cleaning up..."
    rm -rf "$TEST_DIR"
    if [ -d "$WORKTREE_DIR" ]; then
        cd "$PROJECT_ROOT"
        git worktree remove "$WORKTREE_DIR" --force 2>/dev/null || rm -rf "$WORKTREE_DIR"
    fi
}
trap cleanup EXIT

echo "=== Transaction Batching Benchmark ==="
echo "Testing batch flush with $FILE_COUNT files, $RUNS runs each"
echo ""

# Function to create test files
create_test_files() {
    local count=$1
    local dir=$2
    rm -rf "$dir"
    mkdir -p "$dir"

    for i in $(seq 1 $count); do
        cat > "$dir/file_$i.rs" << EOF
pub struct Struct$i { field: i32 }
impl Struct$i {
    pub fn new() -> Self { Self { field: $i } }
    pub fn process(&self) -> i32 { self.field * 2 }
}
pub fn helper_$i() -> i32 { $i }
EOF
    done
}

echo "=== Phase 1: Build NEW implementation ==="
cd "$PROJECT_ROOT"
cargo build --release --quiet
echo "Built: $RKT_NEW"

echo ""
echo "=== Phase 2: Build OLD implementation via git worktree ==="

# Find the commit before transaction batching was added
OLD_COMMIT=$(git log --oneline --all | grep -m1 "auto-refresh and transaction" | awk '{print $1}')
if [ -z "$OLD_COMMIT" ]; then
    # Fallback: use HEAD~10 if we can't find the specific commit
    OLD_COMMIT="HEAD~10"
fi
OLD_COMMIT="${OLD_COMMIT}^"  # Parent of the transaction batching commit

echo "Using old commit: $OLD_COMMIT"

# Create worktree for old version
rm -rf "$WORKTREE_DIR"
git worktree add "$WORKTREE_DIR" "$OLD_COMMIT" --quiet 2>/dev/null || {
    echo "Warning: Could not create worktree, using detached HEAD"
    git worktree add --detach "$WORKTREE_DIR" "$OLD_COMMIT"
}

# Build old version
cd "$WORKTREE_DIR"
cargo build --release --quiet 2>&1 || {
    echo "ERROR: Failed to build old version"
    exit 1
}
echo "Built: $RKT_OLD"

# Create test files
echo ""
echo "Creating $FILE_COUNT test files..."
create_test_files $FILE_COUNT "$TEST_DIR"

echo ""
echo "=== Phase 3: Benchmark NEW implementation (with transaction batching) ==="

NEW_TIMES=""
for run in $(seq 1 $RUNS); do
    cd "$TEST_DIR"
    rm -rf .rocketindex

    # Create fresh index
    "$RKT_NEW" index --quiet > /dev/null 2>&1

    # Modify all files
    for f in *.rs; do
        echo "// modified run $run" >> "$f"
    done

    # Time the refresh (auto-refresh triggers batch flush)
    start=$(python3 -c 'import time; print(time.time())')
    "$RKT_NEW" def "Struct1" --quiet > /dev/null 2>&1
    end=$(python3 -c 'import time; print(time.time())')

    ms=$(python3 -c "print(f'{($end - $start)*1000:.1f}')")
    NEW_TIMES="$NEW_TIMES $ms"
    echo "  Run $run: ${ms}ms"
done

NEW_AVG=$(echo $NEW_TIMES | tr ' ' '\n' | grep -v '^$' | python3 -c "import sys; nums=[float(x) for x in sys.stdin.read().split()]; print(f'{sum(nums)/len(nums):.1f}')")
echo "  Average: ${NEW_AVG}ms"

echo ""
echo "=== Phase 4: Benchmark OLD implementation (via watch mode) ==="
echo "Note: Old code doesn't have auto-refresh, using watch mode simulation"

OLD_TIMES=""
for run in $(seq 1 $RUNS); do
    cd "$TEST_DIR"
    rm -rf .rocketindex

    # Create fresh index
    "$RKT_OLD" index --quiet > /dev/null 2>&1

    # Start watch, modify files, wait for processing
    "$RKT_OLD" watch &
    WATCH_PID=$!
    sleep 0.5

    # Modify all files
    start=$(python3 -c 'import time; print(time.time())')
    for f in *.rs; do
        echo "// old modified run $run" >> "$f"
    done

    # Wait for watch to process (it batches with 100ms window + processing time)
    sleep 1.5
    end=$(python3 -c 'import time; print(time.time())')

    kill $WATCH_PID 2>/dev/null || true
    wait $WATCH_PID 2>/dev/null || true

    # Check if symbols were updated
    count=$("$RKT_OLD" symbols "Struct*" --quiet 2>/dev/null | grep -c "Struct" || echo "0")

    ms=$(python3 -c "print(f'{($end - $start)*1000:.1f}')")
    OLD_TIMES="$OLD_TIMES $ms"
    echo "  Run $run: ${ms}ms (symbols: $count)"
done

OLD_AVG=$(echo $OLD_TIMES | tr ' ' '\n' | grep -v '^$' | python3 -c "import sys; nums=[float(x) for x in sys.stdin.read().split()]; print(f'{sum(nums)/len(nums):.1f}')")
echo "  Average: ${OLD_AVG}ms"

echo ""
echo "=== RESULTS ==="
echo ""
echo "Old (watch mode):     ${OLD_AVG}ms"
echo "New (auto-refresh):   ${NEW_AVG}ms"
echo ""
echo "Note: Old uses watch mode which has additional overhead (file watching, debouncing)."
echo "The transaction batching improvement is in the batch flush operation itself."

echo ""
echo "Benchmark complete."
