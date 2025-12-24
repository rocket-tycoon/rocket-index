#!/bin/bash
# Test all 12 languages

set -e

cd "$(dirname "$0")/.."

echo "===== Rocket Index Claude Plugin Integration Tests ====="
echo

# Create results directory
mkdir -p results

# All 12 supported languages
LANGUAGES=(
    "fsharp-sample"
    "ruby-sample"
    "python-sample"
    "rust-sample"
    "go-sample"
    "java-sample"
    "javascript-sample"
    "typescript-sample"
    "csharp-sample"
    "php-sample"
    "c-sample"
    "cpp-sample"
)

PASS=0
FAIL=0

for lang in "${LANGUAGES[@]}"; do
    echo "Testing $lang..."
    if ./scripts/test-language.sh "$lang"; then
        ((PASS++))
        echo "  ✓ PASS"
    else
        ((FAIL++))
        echo "  ✗ FAIL"
    fi
    echo
done

echo "===== Test Summary ====="
echo "Passed: $PASS/$((PASS + FAIL))"
echo "Failed: $FAIL/$((PASS + FAIL))"
echo
echo "Results saved in results/"
echo
echo "Next steps:"
echo "1. Review results/*.json files"
echo "2. Manually verify tool usage and accuracy"
echo "3. Run: python3 scripts/analyze-results.py"
