#!/bin/bash
# Setup test fixtures for integration testing
# Usage: ./scripts/setup-test-fixtures.sh [ci|full|minimal]
#
# Categories:
#   ci      - Small repos with shallow clones (default, for CI)
#   full    - Large repos for comprehensive testing
#   minimal - Create synthetic minimal fixtures

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
FIXTURES_DIR="$ROOT_DIR/tests/fixtures"
TEST_REPOS_DIR="$ROOT_DIR/test-repos"

CATEGORY="${1:-ci}"

echo "Setting up test fixtures: $CATEGORY"
echo "================================================"

# Clone or update a repo
clone_repo() {
    local name="$1"
    local repo="$2"
    local dest="$3"
    local commit="${4:-}"
    local depth="${5:-}"
    local sparse="${6:-}"

    if [ -d "$dest" ]; then
        echo "  ✓ $name already exists, updating..."
        cd "$dest"
        git fetch --quiet
        if [ -n "$commit" ]; then
            git checkout --quiet "$commit" 2>/dev/null || git checkout --quiet "tags/$commit" 2>/dev/null || true
        fi
        cd - > /dev/null
    else
        echo "  ↓ Cloning $name..."
        mkdir -p "$(dirname "$dest")"

        local clone_args=""
        if [ -n "$depth" ]; then
            clone_args="$clone_args --depth $depth"
        fi

        if [ -n "$sparse" ]; then
            # Sparse checkout for large repos
            git clone --filter=blob:none --sparse $clone_args "$repo" "$dest"
            cd "$dest"
            git sparse-checkout set "$sparse"
            cd - > /dev/null
        else
            git clone $clone_args "$repo" "$dest"
        fi

        if [ -n "$commit" ]; then
            cd "$dest"
            git checkout --quiet "$commit" 2>/dev/null || git checkout --quiet "tags/$commit" 2>/dev/null || true
            cd - > /dev/null
        fi
    fi
}

# Create minimal synthetic fixtures
setup_minimal() {
    echo ""
    echo "Creating minimal synthetic fixtures..."

    # Rust minimal fixture
    local rust_dir="$FIXTURES_DIR/minimal/rust"
    mkdir -p "$rust_dir/src"
    cat > "$rust_dir/src/lib.rs" << 'EOF'
//! Minimal Rust fixture for testing

pub mod utils {
    pub fn helper() -> i32 {
        42
    }
}

pub fn main_function() {
    let x = utils::helper();
    println!("{}", x);
}

pub fn caller_a() {
    main_function();
}

pub fn caller_b() {
    main_function();
    utils::helper();
}

pub struct MyStruct {
    pub field: i32,
}

impl MyStruct {
    pub fn new() -> Self {
        Self { field: utils::helper() }
    }

    pub fn method(&self) -> i32 {
        self.field
    }
}

pub trait MyTrait {
    fn trait_method(&self);
}

impl MyTrait for MyStruct {
    fn trait_method(&self) {
        main_function();
    }
}
EOF
    echo "  ✓ Created $rust_dir"

    # Python minimal fixture
    local python_dir="$FIXTURES_DIR/minimal/python"
    mkdir -p "$python_dir"
    cat > "$python_dir/main.py" << 'EOF'
"""Minimal Python fixture for testing"""

def helper():
    """A helper function"""
    return 42

def main_function():
    """Main entry point"""
    x = helper()
    print(x)

def caller_a():
    """Calls main_function"""
    main_function()

def caller_b():
    """Calls both functions"""
    main_function()
    helper()

class MyClass:
    """A simple class"""

    def __init__(self):
        self.field = helper()

    def method(self):
        return self.field

class ChildClass(MyClass):
    """Inherits from MyClass"""

    def method(self):
        main_function()
        return super().method()
EOF
    echo "  ✓ Created $python_dir"

    # TypeScript minimal fixture
    local ts_dir="$FIXTURES_DIR/minimal/typescript"
    mkdir -p "$ts_dir/src"
    cat > "$ts_dir/src/index.ts" << 'EOF'
// Minimal TypeScript fixture for testing

export function helper(): number {
    return 42;
}

export function mainFunction(): void {
    const x = helper();
    console.log(x);
}

export function callerA(): void {
    mainFunction();
}

export function callerB(): void {
    mainFunction();
    helper();
}

export interface MyInterface {
    method(): number;
}

export class MyClass implements MyInterface {
    private field: number;

    constructor() {
        this.field = helper();
    }

    method(): number {
        return this.field;
    }
}

export class ChildClass extends MyClass {
    method(): number {
        mainFunction();
        return super.method();
    }
}
EOF
    echo "  ✓ Created $ts_dir"
}

# Setup CI fixtures (small, pinned repos)
setup_ci() {
    echo ""
    echo "Setting up CI fixtures (shallow clones, pinned commits)..."

    local ci_dir="$FIXTURES_DIR/ci"
    mkdir -p "$ci_dir"

    # Rust
    clone_repo "serde" "https://github.com/serde-rs/serde" "$ci_dir/rust/serde" "v1.0.215" "1"

    # Python
    clone_repo "click" "https://github.com/pallets/click" "$ci_dir/python/click" "8.1.7" "1"

    # TypeScript
    clone_repo "zod" "https://github.com/colinhacks/zod" "$ci_dir/typescript/zod" "v3.23.8" "1"

    # Go
    clone_repo "cobra" "https://github.com/spf13/cobra" "$ci_dir/go/cobra" "v1.8.1" "1"

    # Ruby
    clone_repo "devise" "https://github.com/heartcombo/devise" "$ci_dir/ruby/devise" "v4.9.4" "1"

    # JavaScript
    clone_repo "lodash" "https://github.com/lodash/lodash" "$ci_dir/javascript/lodash" "4.17.21" "1"

    # Java
    clone_repo "guava" "https://github.com/google/guava" "$ci_dir/java/guava" "v33.3.1" "1"

    # C#
    clone_repo "Polly" "https://github.com/App-vNext/Polly" "$ci_dir/csharp/Polly" "8.5.0" "1"

    # F#
    clone_repo "FsUnit" "https://github.com/fsprojects/FsUnit" "$ci_dir/fsharp/FsUnit" "v6.0.1" "1"

    # C
    clone_repo "redis" "https://github.com/redis/redis" "$ci_dir/c/redis" "7.4.1" "1"

    # C++
    clone_repo "json" "https://github.com/nlohmann/json" "$ci_dir/cpp/json" "v3.11.3" "1"
}

# Setup full fixtures (uses existing test-repos)
setup_full() {
    echo ""
    echo "Full fixtures use existing test-repos/ directory"
    echo "Run manually to clone large repos:"
    echo ""
    echo "  Rust:   git clone https://github.com/tokio-rs/tokio test-repos/rust/tokio"
    echo "  Python: git clone https://github.com/django/django test-repos/python/django"
    echo "  Go:     git clone https://github.com/moby/moby test-repos/go/docker"
    echo "  Ruby:   git clone https://github.com/rails/rails test-repos/ruby/rails"
}

# Main
case "$CATEGORY" in
    minimal)
        setup_minimal
        ;;
    ci)
        setup_minimal
        setup_ci
        ;;
    full)
        setup_minimal
        setup_ci
        setup_full
        ;;
    *)
        echo "Unknown category: $CATEGORY"
        echo "Usage: $0 [ci|full|minimal]"
        exit 1
        ;;
esac

echo ""
echo "================================================"
echo "Done! Fixtures ready in: $FIXTURES_DIR"
