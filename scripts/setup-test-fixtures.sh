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

/// OtherStruct for testing disambiguation of common names
pub struct OtherStruct {
    value: String,
}

impl OtherStruct {
    pub fn new(s: &str) -> Self {
        Self { value: s.to_string() }
    }

    pub fn init(&mut self) {
        self.value = "initialized".to_string();
    }

    pub fn run(&self) {
        println!("{}", self.value);
    }
}
EOF

    # Rust cross-file caller with cycle detection
    cat > "$rust_dir/src/caller.rs" << 'EOF'
//! Cross-file caller module for testing find_callers across files

use crate::{main_function, utils};

pub fn cross_file_caller() {
    main_function();
    utils::helper();
}

pub fn another_caller() {
    cross_file_caller();
}

/// Mutually recursive functions for cycle detection testing
pub fn cycle_a(n: i32) {
    if n > 0 {
        cycle_b(n - 1);
    }
}

pub fn cycle_b(n: i32) {
    if n > 0 {
        cycle_a(n - 1);
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

class OtherClass:
    """Another class for testing disambiguation"""

    def __init__(self, value):
        self.value = value

    def init(self):
        """Common method name for testing"""
        self.value = "initialized"

    def run(self):
        """Common method name for testing"""
        print(self.value)
EOF

    # Python cross-file caller
    cat > "$python_dir/caller.py" << 'EOF'
"""Cross-file caller for testing find_callers across files"""
from main import main_function, helper

def cross_file_caller():
    """Calls main_function from another file"""
    main_function()
    helper()

def another_caller():
    """Calls cross_file_caller"""
    cross_file_caller()
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

export class OtherClass {
    private value: string;

    constructor(value: string) {
        this.value = value;
    }

    init(): void {
        this.value = "initialized";
    }

    run(): void {
        console.log(this.value);
    }
}
EOF

    # TypeScript cross-file caller
    cat > "$ts_dir/src/caller.ts" << 'EOF'
// Cross-file caller for testing find_callers across files
import { mainFunction, helper } from './index';

export function crossFileCaller(): void {
    mainFunction();
    helper();
}

export function anotherCaller(): void {
    crossFileCaller();
}
EOF
    echo "  ✓ Created $ts_dir"

    # Go minimal fixture
    local go_dir="$FIXTURES_DIR/minimal/go"
    mkdir -p "$go_dir"
    cat > "$go_dir/main.go" << 'EOF'
package main

import "fmt"

func helper() int {
    return 42
}

func mainFunction() {
    x := helper()
    fmt.Println(x)
}

func callerA() {
    mainFunction()
}

func callerB() {
    mainFunction()
    helper()
}

type MyStruct struct {
    Field int
}

func NewMyStruct() *MyStruct {
    return &MyStruct{Field: helper()}
}

func (m *MyStruct) Method() int {
    return m.Field
}
EOF
    echo "  ✓ Created $go_dir"

    # Ruby minimal fixture
    local ruby_dir="$FIXTURES_DIR/minimal/ruby"
    mkdir -p "$ruby_dir"
    cat > "$ruby_dir/main.rb" << 'EOF'
# Minimal Ruby fixture for testing

def helper
  42
end

def main_function
  x = helper
  puts x
end

def caller_a
  main_function
end

def caller_b
  main_function
  helper
end

class MyClass
  attr_reader :field

  def initialize
    @field = helper
  end

  def method
    @field
  end
end

class ChildClass < MyClass
  def method
    main_function
    super
  end
end
EOF
    echo "  ✓ Created $ruby_dir"

    # Java minimal fixture
    local java_dir="$FIXTURES_DIR/minimal/java"
    mkdir -p "$java_dir"
    cat > "$java_dir/Main.java" << 'EOF'
package minimal;

public class Main {
    public static int helper() {
        return 42;
    }

    public static void mainFunction() {
        int x = helper();
        System.out.println(x);
    }

    public static void callerA() {
        mainFunction();
    }

    public static void callerB() {
        mainFunction();
        helper();
    }
}

class MyClass {
    private int field;

    public MyClass() {
        this.field = Main.helper();
    }

    public int method() {
        return this.field;
    }
}

class ChildClass extends MyClass {
    @Override
    public int method() {
        Main.mainFunction();
        return super.method();
    }
}
EOF
    echo "  ✓ Created $java_dir"

    # JavaScript minimal fixture
    local js_dir="$FIXTURES_DIR/minimal/javascript"
    mkdir -p "$js_dir"
    cat > "$js_dir/main.js" << 'EOF'
// Minimal JavaScript fixture for testing

function helper() {
    return 42;
}

function mainFunction() {
    const x = helper();
    console.log(x);
}

function callerA() {
    mainFunction();
}

function callerB() {
    mainFunction();
    helper();
}

class MyClass {
    constructor() {
        this.field = helper();
    }

    method() {
        return this.field;
    }
}

class ChildClass extends MyClass {
    method() {
        mainFunction();
        return super.method();
    }
}

module.exports = { helper, mainFunction, callerA, callerB, MyClass, ChildClass };
EOF
    echo "  ✓ Created $js_dir"

    # C# minimal fixture
    local csharp_dir="$FIXTURES_DIR/minimal/csharp"
    mkdir -p "$csharp_dir"
    cat > "$csharp_dir/Main.cs" << 'EOF'
namespace Minimal;

public static class Helpers
{
    public static int Helper()
    {
        return 42;
    }
}

public class Program
{
    public static void MainFunction()
    {
        var x = Helpers.Helper();
        Console.WriteLine(x);
    }

    public static void CallerA()
    {
        MainFunction();
    }

    public static void CallerB()
    {
        MainFunction();
        Helpers.Helper();
    }
}

public class MyClass
{
    private int _field;

    public MyClass()
    {
        _field = Helpers.Helper();
    }

    public virtual int Method()
    {
        return _field;
    }
}

public class ChildClass : MyClass
{
    public override int Method()
    {
        Program.MainFunction();
        return base.Method();
    }
}
EOF
    echo "  ✓ Created $csharp_dir"

    # F# minimal fixture
    local fsharp_dir="$FIXTURES_DIR/minimal/fsharp"
    mkdir -p "$fsharp_dir"
    cat > "$fsharp_dir/Main.fs" << 'EOF'
module Minimal.Main

let helper () = 42

let mainFunction () =
    let x = helper ()
    printfn "%d" x

let callerA () =
    mainFunction ()

let callerB () =
    mainFunction ()
    helper () |> ignore

type MyClass() =
    let field = helper ()
    member _.Method() = field

type ChildClass() =
    inherit MyClass()
    override _.Method() =
        mainFunction ()
        base.Method()
EOF
    echo "  ✓ Created $fsharp_dir"

    # PHP minimal fixture
    local php_dir="$FIXTURES_DIR/minimal/php"
    mkdir -p "$php_dir"
    cat > "$php_dir/main.php" << 'EOF'
<?php

function helper(): int {
    return 42;
}

function mainFunction(): void {
    $x = helper();
    echo $x;
}

function callerA(): void {
    mainFunction();
}

function callerB(): void {
    mainFunction();
    helper();
}

class MyClass {
    private int $field;

    public function __construct() {
        $this->field = helper();
    }

    public function method(): int {
        return $this->field;
    }
}

class ChildClass extends MyClass {
    public function method(): int {
        mainFunction();
        return parent::method();
    }
}
EOF
    echo "  ✓ Created $php_dir"

    # C minimal fixture
    local c_dir="$FIXTURES_DIR/minimal/c"
    mkdir -p "$c_dir"
    cat > "$c_dir/main.c" << 'EOF'
#include <stdio.h>

int helper(void) {
    return 42;
}

void main_function(void) {
    int x = helper();
    printf("%d\n", x);
}

void caller_a(void) {
    main_function();
}

void caller_b(void) {
    main_function();
    helper();
}

typedef struct {
    int field;
} MyStruct;

MyStruct* my_struct_new(void) {
    static MyStruct s;
    s.field = helper();
    return &s;
}

int my_struct_method(MyStruct* self) {
    return self->field;
}
EOF
    echo "  ✓ Created $c_dir"

    # C++ minimal fixture
    local cpp_dir="$FIXTURES_DIR/minimal/cpp"
    mkdir -p "$cpp_dir"
    cat > "$cpp_dir/main.cpp" << 'EOF'
#include <iostream>

int helper() {
    return 42;
}

void mainFunction() {
    int x = helper();
    std::cout << x << std::endl;
}

void callerA() {
    mainFunction();
}

void callerB() {
    mainFunction();
    helper();
}

class MyClass {
private:
    int field;
public:
    MyClass() : field(helper()) {}
    virtual int method() { return field; }
};

class ChildClass : public MyClass {
public:
    int method() override {
        mainFunction();
        return MyClass::method();
    }
};
EOF
    echo "  ✓ Created $cpp_dir"
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
