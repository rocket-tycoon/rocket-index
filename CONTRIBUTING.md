# Contributing to RocketIndex

This document describes the process for contributing to RocketIndex. It is based on the [C4 (Collective Code Construction Contract)](https://rfc.zeromq.org/spec/42/).

## Goals

- Provide a simple, efficient process for contributing
- Maximize the scale of the community by reducing friction for new contributors
- Support effective parallel development through a model of strong positive feedback

## Licensing

RocketIndex uses the Business Source License 1.1 (BSL), which converts to Apache 2.0 after four years. By contributing, you agree that your contributions will be licensed under the same terms. You retain copyright to your contributions.

## Development Process

### Prerequisites

- Rust (stable toolchain)
- Git

### Getting Started

```bash
# Fork and clone the repository
git clone https://github.com/YOUR_USERNAME/rocket-index.git
cd rocket-index

# Build
cargo build --release

# Run tests
cargo test --all

# Check lints
cargo clippy
cargo fmt --check
```

### Making Changes

1. **Log an Issue First**
   - Problems must be logged as issues before patches are submitted
   - Describe the observable problem (bug, missing feature, etc.)
   - One issue per problem

2. **Fork and Branch**
   - Fork the repository
   - Create a branch for your change
   - Branch names should be descriptive (e.g., `fix-symbol-resolution`, `add-go-support`)

3. **Write Your Patch**
   - One patch solves one problem
   - Patches must compile cleanly and pass all tests
   - Follow the coding style in `coding-guidelines.md`
   - Include tests for new functionality
   - Keep commits atomic and focused

4. **Submit a Pull Request**
   - Reference the issue number in the PR description
   - Describe what the patch does
   - PRs should be minimal—avoid unrelated changes

### Patch Requirements

A correct patch:

- Solves exactly one identified problem
- Compiles without errors or warnings
- Passes all existing tests
- Includes tests for new behavior (where applicable)
- Follows the project's coding style
- Does not break existing functionality

### What Maintainers Do

Maintainers merge correct patches. They do not make value judgments beyond correctness. A patch that meets the requirements above will be merged.

Maintainers should:
- Merge correct patches rapidly
- Not cherry-pick or rewrite patches
- Not reject patches based on personal preference

## Community Standards

### Expected Behavior

- Focus on the work
- Be respectful and professional
- Assume good faith
- Provide constructive feedback on patches

### Bad Actors

Administrators may block or ban contributors who:
- Repeatedly ignore the rules and culture of the project
- Are needlessly argumentative or hostile
- Cause stress and disruption to others
- Cannot self-correct when asked

This is done after public discussion, with opportunity for all parties to speak. The standard is behavioral, not ideological—we don't care what you believe, only how you act.

## Getting Help

- Open an issue for questions about the codebase
- Check existing issues before opening new ones
- Read the [README](README.md) and [CLAUDE.md](CLAUDE.md) for project context

## Summary

1. Log an issue describing the problem
2. Fork, branch, write a minimal patch
3. Ensure it compiles, passes tests, follows style
4. Submit a PR referencing the issue
5. Maintainers merge correct patches

That's it. No CLAs, no committees, no bureaucracy. Just code.
