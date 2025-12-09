# RocketIndex Benchmark Results

*Generated: 2025-12-09 10:56*

## Summary

| Language | Model | Avg Turn Reduction | Success Rate (with RKT) | Tasks |
|----------|-------|-------------------|-------------------------|-------|
| Ruby | Haiku | +36% | 100% | 2 |
| Ruby | Sonnet | +7% | 100% | 2 |
| Rust | Haiku | +50% | 100% | 1 |
| Rust | Sonnet | +57% | 100% | 1 |

## Key Findings

### Haiku Model Uplift
- Best improvement: **+73%** turn reduction on `find_callers_factory` (ruby)
- RocketIndex enables Haiku to complete tasks it would otherwise fail

### Failure Prevention
- RocketIndex prevented 1 task failure(s):
  - ruby/haiku: `find_callers_factory`


## Ruby Results

### Haiku

| Task | Category | Without RKT | With RKT | Turn Reduction |
|------|----------|-------------|----------|----------------|
| find_callers_factory | find_callers | 15 turns (0% success) | 4 turns | +73% |
| find_definition_factory | find_definition | 2.0 turns | 2.0 turns | +0% |

### Sonnet

| Task | Category | Without RKT | With RKT | Turn Reduction |
|------|----------|-------------|----------|----------------|
| find_callers_factory | find_callers | 14 turns | 12 turns | +14% |
| find_definition_factory | find_definition | 2.0 turns | 2.0 turns | +0% |

## Rust Results

### Haiku

| Task | Category | Without RKT | With RKT | Turn Reduction |
|------|----------|-------------|----------|----------------|
| find_definition_spawn | find_definition | 6.0 turns | 3.0 turns | +50% |

### Sonnet

| Task | Category | Without RKT | With RKT | Turn Reduction |
|------|----------|-------------|----------|----------------|
| find_definition_spawn | find_definition | 7.0 turns | 3.0 turns | +57% |


## Reproduction

```bash
# Index the repository first
cd /path/to/repo && rkt index

# Run benchmarks
./scripts/benchmarks/run_benchmark.sh \
  --task-file tasks/ruby_vets_api.json \
  --model haiku

# Aggregate results
python3 scripts/benchmarks/aggregate_results.py \
  --input scripts/benchmarks/results/ \
  --output docs/benchmarks/results.md
```