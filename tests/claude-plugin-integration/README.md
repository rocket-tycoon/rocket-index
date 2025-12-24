# Claude Plugin Integration Tests for Rocket Index

Automated testing of Rocket Index as a Claude Code plugin across all 12 supported languages.

## Test Structure

```
tests/claude-plugin-integration/
├── test-repos/          # Sample codebases for each language
│   ├── ruby-sample/
│   ├── python-sample/
│   ├── typescript-sample/
│   ├── rust-sample/
│   ├── go-sample/
│   ├── java-sample/
│   ├── javascript-sample/
│   ├── csharp-sample/
│   ├── fsharp-sample/
│   ├── php-sample/
│   ├── c-sample/
│   └── cpp-sample/
├── scripts/             # Test automation
│   ├── test-language.sh   # Test single language
│   ├── test-all.sh        # Test all languages
│   └── analyze-results.py # Analyze results
└── results/             # Test output (JSON)
```

## Quick Start

### Test All Languages

```bash
cd tests/claude-plugin-integration
./scripts/test-all.sh
```

### Test Single Language

```bash
./scripts/test-language.sh ruby-sample
```

### Analyze Results

```bash
python3 scripts/analyze-results.py
```

## What's Being Tested

For each language, Claude Code is asked to:

1. **Find definition** - Locate the `User` class/type
2. **Find callers** - Find all functions that call `process_payment`
3. **Search symbols** - Search for symbols matching `Payment*`

## Success Criteria

-  **Tool Usage**: Claude uses Rocket Index MCP tools (not grep)
- ✅ **Correctness**: Results match actual code locations
- ✅ **Completeness**: All callers/references found
- ✅ **Response Quality**: Clear, structured output

## Test Repositories

Each test repo contains:
- 3 files (user, payment service, main)
- Classes/types with methods
- Function calls between modules
- Import/open statements

All repos follow the same pattern for consistency.

## Manual Review

After running tests, review `results/*.json` files:

1. **Did Claude use Rocket Index tools?**
   - Look for `find_definition`, `find_callers`, etc.
   - Check for absence of grep/file reading

2. **Were results accurate?**
   - Compare file paths and line numbers to actual code
   - Verify all callers were found

3. **Was output quality good?**
   - Clear explanations
   - Structured JSON
   - Correct symbol names

## Notes

- Tests use Claude Code headless mode (`claude -p`)
- Rocket Index must be installed and available as `rkt` command
- Each repo directory is indexed before testing
- Results are JSON for programmatic analysis
