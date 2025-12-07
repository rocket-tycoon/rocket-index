//! Embedded coding guidelines templates for Claude Code integration
//!
//! Language-specific coding guidelines that can be installed during setup.
//! Each derives from a base set of AI Code Assistance principles, customized
//! for language idioms and ecosystem conventions.

/// A coding guidelines template for a specific language
#[allow(dead_code)]
pub struct Guidelines {
    /// Language identifier (e.g., "rust", "fsharp")
    pub language: &'static str,
    /// Display name for selection UI (e.g., "Rust", "F#")
    pub display_name: &'static str,
    /// File extensions to detect this language
    pub extensions: &'static [&'static str],
    /// Full coding-guidelines.md content
    pub content: &'static str,
}

/// All available coding guidelines
#[allow(dead_code)]
pub const GUIDELINES: &[Guidelines] = &[
    Guidelines {
        language: "rust",
        display_name: "Rust",
        extensions: &["rs"],
        content: r#"# Rust AI Code Assistance Guidelines

## Core Principles

- **Make Invalid States Unrepresentable**: Use the type system (Enums, Newtypes) to ensure data is correct by definition, not just by convention.
- **Explicit over Implicit**: Rust favors explicit error handling, conversion, and memory management. Avoid "magic" logic.
- **Fight Complexity, Not the Borrow Checker**: If a solution requires complex lifetime annotations (`'a`) for simple logic, refactor the data structure or use `.clone()`/`Arc` until performance proves it's a bottleneck.
- **Safety Absolute**: Never suggest `unsafe` blocks unless explicitly requested for FFI or low-level systems programming.
- **Search Before Building**: Most problems aren't novel. Check crates.io and the standard library before writing custom solutions.

## Implementation & Workflow

- **Model Data First**: Define `struct`s and `enum`s before writing logic. The shape of the data dictates the flow of the application.
- **Handle Errors Immediately**: Do not use `.unwrap()` in production code. Use the `?` operator for propagation or `match` for handling.
  - `.expect("descriptive message")` is acceptable when the invariant is truly guaranteed by preceding logic.
  - **Prototyping Exception**: `.unwrap()` is acceptable only if explicitly marked with a `// TODO: Handle error` comment.
- **Implement Only What's Asked**: No extra features or future-proofing unless requested.
- **Start with Happy Path**: Handle edge cases incrementally unless security concerns demand otherwise.
- **Clippy Compliance**: Code must be lint-free according to `cargo clippy`.

## Code Structure

- **Limit Nesting**: Keep conditionals/loops under 3 layers.
- **Function Length**: 25-30 lines max; break up longer functions.
- **Idiomatic Control Flow**: Prefer `match` and `if let` over complex boolean logic chains.
- **Iterators over Loops**: Prefer functional combinators (`.map()`, `.filter()`, `.collect()`) over manual `for` loops.
- **Newtype Pattern**: Avoid "Primitive Obsession." Don't pass `String` for an ID; create `struct UserId(String);`.
- **Modules**: Group code by functionality (feature-based). Keep `main.rs` and `lib.rs` clean.

## Best Practices

- **Parse, Don't Validate**: Write functions that parse data and return `Result<ValidatedType, Error>`, not functions that check and return bool.
- **Error Types**:
  - Use `anyhow::Result` for **Applications** (easy error propagation with context).
  - Use `thiserror` (custom Enums) for **Libraries** (explicit, typed error API).
- **Standard Ecosystem**: Use established crates (`serde`, `tokio`, `clap`, `reqwest`, `tracing`) rather than reinventing.
- **Secrets & Config**: Never hardcode credentials. Use `dotenvy` or `config` crates.
- **`#[must_use]`**: Apply to functions where ignoring the return value is almost certainly a bug.
- **Derive Macros**: Liberally use `#[derive(Debug, Clone, PartialEq)]`.

## Testing

- **Unit Tests**: Place in same file inside `#[cfg(test)] mod tests { ... }`.
- **Documentation Tests**: Include executable examples in `///` doc comments.
- **Integration Tests**: Place in `tests/` directory.
- **Property Testing**: Use `proptest` or `quickcheck` for invariant-based testing.

## Response Approach

- **Type Signature First**: Show function signatures and struct definitions before implementation.
- **Explain Ownership**: If using `.clone()`, `Rc`, `Arc`, `Mutex`, or `Cow`, briefly explain why.
- **Compilable Code**: All Rust snippets must be syntactically correct.

---

Remember: Rust rewards thoughtful ownership design. Write code that leverages the type system to make correctness obvious and memory safety guaranteed.
"#,
    },
    Guidelines {
        language: "fsharp",
        display_name: "F#",
        extensions: &["fs", "fsx", "fsi"],
        content: r#"# F# AI Code Assistance Guidelines

## Core Principles

- **Simplicity First**: Generate the most direct solution that meets requirements.
- **Let the Type System Work**: Leverage F#'s type inference and algebraic data types to make invalid states unrepresentable.
- **Explicit Code**: Write straightforward code; favor readability over cleverness.
- **Search Before Building**: Most problems aren't novel. Check FSharp.Core and established packages before writing custom solutions.

## Implementation

- **Implement Only What's Asked**: No extra features or future-proofing unless requested.
- **Types as Contracts**: Define discriminated unions and record types to model your domain before implementation.
- **Start with Happy Path**: Use the Result type for explicit error paths; handle edge cases incrementally.
- **Lean Code**: Skip retry logic and other complexity unless explicitly needed.
- **Ask About Backwards Compatibility**: Always inquire rather than assume.

## Code Structure

- **Limit Nesting**: Prefer pattern matching and pipelines over deeply nested conditionals; max 2-3 levels.
- **Function Length**: 15-25 lines max; F#'s expressiveness usually allows shorter functions.
- **Pure by Default**: Write pure functions; isolate side effects at the edges.
- **Concrete Over Abstract**: Avoid abstraction (interfaces, object hierarchies) unless it adds real value.
- **Composition Over Inheritance**: Use function composition (>>) and pipelines (|>).
- **Module-First Organization**: Group related functions, types, and values in modules.

## F# Idioms

- **Prefer Immutability**: Use let bindings and immutable records; mutable only when performance demands.
- **Use Discriminated Unions**: Model domain states, errors, and variants with DUs rather than exceptions.
- **Pipeline Style**: Structure transformations as `input |> step1 |> step2 |> step3`.
- **Pattern Matching**: Prefer match expressions over if-else chains.
- **Option Over Null**: Use `Option<'T>` for values that may be absent.
- **Result for Errors**: Use `Result<'T, 'E>` for operations that can fail; reserve exceptions for truly exceptional circumstances.
- **Partial Application**: Design functions with "most stable" arguments first.

## Best Practices

- **Choose Right Tools**: Use FSharp.Core and BCL when sufficient; add packages when they save significant time.
- **Validate at Boundaries**: Parse external inputs into domain types as early as possible.
- **Secrets Management**: NEVER commit secrets. Use environment variables or secure vaults.
- **Early Return via Pattern Matching**: Use guard clauses and pattern matching for invalid cases upfront.

## Testing

- **Test-Driven Development**: Write tests first when requirements are clear.
- **Property-Based Testing**: Consider FsCheck for testing invariants.
- **Tests as Specifications**: Structure tests to articulate what the code should do.
- **Test Pure Functions**: F#'s emphasis on purity makes unit testing straightforward.

## What to Avoid

- **Overusing Classes**: Prefer modules with functions; use classes only for interop or resource management.
- **Stringly-Typed Code**: Wrap primitive types in single-case DUs when it improves clarity.
- **Ignoring Warnings**: F# warnings (especially incomplete pattern matches) are often errors waiting to happen.
- **Premature Optimization**: Write clear code first; optimize with evidence from profiling.

---

Remember: F# rewards thoughtful domain modeling and compositional design. Write clean, focused code that leverages the type system to make correctness obvious.
"#,
    },
    Guidelines {
        language: "ruby",
        display_name: "Ruby",
        extensions: &["rb", "rake", "gemspec"],
        content: r#"# Ruby AI Code Assistance Guidelines

## Core Principles

- **Simplicity First**: Generate the most direct solution that meets requirements.
- **Convention Over Configuration**: Follow Ruby and Rails conventions; deviate only with good reason.
- **Readable Code**: Write code that reads like well-written prose. Clarity over cleverness.
- **Search Before Building**: Most problems aren't novel. Check RubyGems and standard library before writing custom solutions.

## Implementation

- **Implement Only What's Asked**: No extra features or future-proofing unless requested.
- **Start with Happy Path**: Handle edge cases incrementally unless security concerns demand otherwise.
- **Lean Code**: Skip complex error handling, retries, and abstractions unless explicitly needed.
- **Duck Typing with Intent**: Rely on duck typing, but use meaningful method names that convey intent.
- **Ask About Backwards Compatibility**: Always inquire rather than assume.

## Code Structure

- **Limit Nesting**: Keep conditionals/loops under 3 layers. Use guard clauses and early returns.
- **Method Length**: 10-15 lines max; Ruby's expressiveness allows short methods.
- **Single Responsibility**: Each class/module should have one reason to change.
- **Composition Over Inheritance**: Prefer modules and composition over deep inheritance hierarchies.
- **Feature-First Organization**: Group by functionality, then by type.

## Ruby Idioms

- **Blocks and Iterators**: Prefer `each`, `map`, `select`, `reduce` over manual loops.
- **Guard Clauses**: Use `return early if condition` instead of wrapping in if blocks.
- **Symbols for Keys**: Use symbols (`:name`) for hash keys, strings for data.
- **Predicate Methods**: Methods returning boolean should end with `?` (e.g., `valid?`).
- **Bang Methods**: Methods with `!` modify in place or raise exceptions.
- **Implicit Returns**: Omit explicit `return` for the last expression.
- **String Interpolation**: Prefer `"Hello, #{name}"` over concatenation.
- **Frozen String Literals**: Add `# frozen_string_literal: true` to files for performance.

## Best Practices

- **RuboCop Compliance**: Code should pass RuboCop linting with project's config.
- **Use Built-ins**: Ruby's standard library is extensive. Check Enumerable, File, Time before adding gems.
- **Validate at Boundaries**: Validate external inputs early; trust internal data.
- **Secrets Management**: NEVER commit secrets. Use environment variables or Rails credentials.
- **Explicit Dependencies**: Keep Gemfile clean; remove unused gems.

## Testing

- **Test-Driven Development**: Write tests first when requirements are clear.
- **RSpec or Minitest**: Follow project conventions. Structure tests with describe/context/it.
- **Tests as Specifications**: Test names should describe behavior: `it "returns nil when user not found"`.
- **Test Levels**: Unit tests for domain logic, integration tests for API/feature behavior.
- **Avoid Over-Mocking**: Mock external services, not internal collaborators.

## Rails-Specific (if applicable)

- **Skinny Controllers**: Keep controllers thin; move logic to models or service objects.
- **Fat Models, Then Extract**: Start with logic in models, extract to concerns/services when it grows.
- **Query Objects**: Extract complex queries into dedicated classes.
- **Strong Parameters**: Always use strong params for user input.
- **ActiveRecord Patterns**: Use scopes, callbacks judiciously; prefer explicit over magic.

## What to Avoid

- **Monkey Patching**: Extend core classes only when absolutely necessary and well-documented.
- **Metaprogramming Abuse**: Use define_method, method_missing sparingly; prefer explicit methods.
- **Global State**: Avoid class variables (@@var) and global variables ($var).
- **Premature Abstraction**: Three similar lines > one premature abstraction.

---

Remember: Ruby is designed for programmer happiness. Write code that is a pleasure to read and maintain.
"#,
    },
    Guidelines {
        language: "typescript",
        display_name: "TypeScript",
        extensions: &["ts", "tsx"],
        content: r#"# TypeScript AI Code Assistance Guidelines

## Core Principles

- **Type Safety First**: Leverage TypeScript's type system to catch errors at compile time.
- **Strict Mode Always**: Enable `strict: true` in tsconfig.json. Never use `any` without explicit justification.
- **Explicit Over Implicit**: Prefer explicit type annotations for function parameters and return types.
- **Search Before Building**: Most problems aren't novel. Check npm and built-in APIs before writing custom solutions.

## Implementation

- **Implement Only What's Asked**: No extra features or future-proofing unless requested.
- **Start with Happy Path**: Handle edge cases incrementally unless security concerns demand otherwise.
- **Lean Code**: Skip complex error handling and abstractions unless explicitly needed.
- **Types as Documentation**: Well-defined types reduce the need for comments.
- **Ask About Backwards Compatibility**: Always inquire rather than assume.

## Code Structure

- **Limit Nesting**: Keep conditionals/loops under 3 layers. Use early returns.
- **Function Length**: 25-30 lines max; break up longer functions.
- **Single Responsibility**: Each module/class should have one clear purpose.
- **Feature-First Organization**: Group by feature, not by type (components/, services/).
- **Barrel Exports**: Use index.ts for clean public APIs, but avoid deep re-exports.

## TypeScript Idioms

- **Discriminated Unions**: Use tagged unions for state modeling:
  ```typescript
  type Result<T> = { ok: true; value: T } | { ok: false; error: Error };
  ```
- **Type Narrowing**: Use type guards and `in` operator for safe narrowing.
- **Utility Types**: Leverage `Partial`, `Required`, `Pick`, `Omit`, `Record`.
- **Const Assertions**: Use `as const` for literal types and readonly arrays.
- **Never for Exhaustiveness**: Use `never` in switch defaults to catch unhandled cases.
- **Optional Chaining**: Use `?.` and `??` for safe property access.
- **Template Literal Types**: For string pattern matching when appropriate.

## Best Practices

- **ESLint + Prettier**: Code should pass linting. Use consistent formatting.
- **Avoid `any`**: Use `unknown` when type is truly unknown, then narrow.
- **Validate at Boundaries**: Parse external data with Zod, io-ts, or similar.
- **Secrets Management**: NEVER commit secrets. Use environment variables.
- **Immutability**: Prefer `readonly` arrays and `Readonly<T>` for data that shouldn't change.
- **Null vs Undefined**: Be consistent. Prefer `undefined` for optional values.

## Testing

- **Test-Driven Development**: Write tests first when requirements are clear.
- **Jest or Vitest**: Follow project conventions. Structure with describe/it.
- **Type Testing**: Use `expectTypeOf` or `tsd` for testing type definitions.
- **Tests as Specifications**: Test names should describe behavior clearly.
- **Integration Tests**: Test API contracts and component interactions.

## React-Specific (if applicable)

- **Functional Components**: Use function components with hooks.
- **Props Types**: Define explicit interfaces for component props.
- **Custom Hooks**: Extract reusable logic into custom hooks.
- **Avoid Over-Rendering**: Use `useMemo`, `useCallback` judiciously, not prematurely.

## What to Avoid

- **Type Assertions Abuse**: Avoid `as` unless you've validated the shape.
- **Enums in Most Cases**: Prefer const objects or union types over enums.
- **Class Over Function**: Prefer functions and closures over classes for most logic.
- **Premature Abstraction**: Three similar lines > one premature abstraction.

---

Remember: TypeScript's value is in catching bugs before runtime. Write code that leverages the type system to make invalid states unrepresentable.
"#,
    },
    Guidelines {
        language: "python",
        display_name: "Python",
        extensions: &["py", "pyi"],
        content: r#"# Python AI Code Assistance Guidelines

## Core Principles

- **Readability Counts**: Write code that is clear and self-documenting. Follow PEP 8.
- **Explicit is Better Than Implicit**: The Zen of Python guides all decisions.
- **Type Hints Always**: Use type hints for function signatures and complex variables.
- **Search Before Building**: Most problems aren't novel. Check PyPI and standard library before writing custom solutions.

## Implementation

- **Implement Only What's Asked**: No extra features or future-proofing unless requested.
- **Start with Happy Path**: Handle edge cases incrementally unless security concerns demand otherwise.
- **Lean Code**: Skip complex error handling and abstractions unless explicitly needed.
- **Use Standard Library**: Python's stdlib is extensive; check before adding dependencies.
- **Ask About Backwards Compatibility**: Always inquire rather than assume.

## Code Structure

- **Limit Nesting**: Keep conditionals/loops under 3 layers. Use guard clauses.
- **Function Length**: 25-30 lines max; break up longer functions.
- **Single Responsibility**: Each module/class should have one clear purpose.
- **Feature-First Organization**: Group by functionality, then by type.
- **Flat is Better Than Nested**: Prefer flat module structures.

## Python Idioms

- **Type Hints**: Use `typing` module for complex types:
  ```python
  def process(items: list[str]) -> dict[str, int]: ...
  ```
- **Dataclasses**: Use `@dataclass` for data containers instead of plain classes.
- **Context Managers**: Use `with` statements for resource management.
- **List/Dict/Set Comprehensions**: Prefer over map/filter for simple transformations.
- **Generator Expressions**: Use for memory-efficient iteration.
- **f-strings**: Prefer `f"Hello, {name}"` over `.format()` or `%`.
- **Walrus Operator**: Use `:=` when it improves clarity.
- **Pathlib**: Use `Path` instead of `os.path` for file operations.

## Best Practices

- **Ruff/Black/Flake8**: Code should pass linting. Use consistent formatting.
- **Type Checking**: Run `mypy` or `pyright` in strict mode.
- **Validate at Boundaries**: Use Pydantic for external data validation.
- **Secrets Management**: NEVER commit secrets. Use environment variables or python-dotenv.
- **Virtual Environments**: Always use venv or poetry for dependency isolation.
- **Requirements Pinning**: Pin exact versions in production.

## Testing

- **Test-Driven Development**: Write tests first when requirements are clear.
- **Pytest**: Prefer pytest over unittest. Use fixtures and parametrize.
- **Tests as Specifications**: Test names should describe behavior: `test_returns_none_when_user_not_found`.
- **Property Testing**: Use Hypothesis for invariant-based testing.
- **Integration Tests**: Test API contracts and component interactions.

## Error Handling

- **Specific Exceptions**: Catch specific exceptions, not bare `except:`.
- **Custom Exceptions**: Define domain exceptions inheriting from `Exception`.
- **EAFP**: "Easier to Ask Forgiveness than Permission" - use try/except over pre-checking.
- **Context in Errors**: Include relevant data in exception messages.

## What to Avoid

- **Mutable Default Arguments**: Never use `def foo(items=[])`. Use `None` and initialize inside.
- **Star Imports**: Never use `from module import *`.
- **Global State**: Avoid module-level mutable state.
- **Bare Except**: Never use `except:` without specifying exception type.
- **Premature Abstraction**: Three similar lines > one premature abstraction.

---

Remember: Python emphasizes readability and simplicity. Write code that is a pleasure to read and maintain.
"#,
    },
    Guidelines {
        language: "go",
        display_name: "Go",
        extensions: &["go"],
        content: r#"# Go AI Code Assistance Guidelines

## Core Principles

- **Simplicity First**: Go favors simplicity over cleverness. If it seems complex, find a simpler way.
- **Explicit Over Implicit**: No magic. No hidden control flow. What you see is what you get.
- **Accept Interfaces, Return Structs**: Functions should accept interfaces and return concrete types.
- **Search Before Building**: Most problems aren't novel. Check the standard library before adding dependencies.

## Implementation

- **Implement Only What's Asked**: No extra features or future-proofing unless requested.
- **Start with Happy Path**: Handle edge cases incrementally unless security concerns demand otherwise.
- **Lean Code**: Skip complex abstractions unless explicitly needed.
- **Standard Library First**: Go's stdlib is comprehensive; use it before reaching for third-party packages.
- **Ask About Backwards Compatibility**: Always inquire rather than assume.

## Code Structure

- **Limit Nesting**: Keep conditionals/loops under 3 layers. Use early returns.
- **Function Length**: 30-40 lines max (Go tends to be more verbose).
- **Package Organization**: One package per directory. Package name matches directory name.
- **Internal Packages**: Use `internal/` for code that shouldn't be imported externally.
- **Flat Structure**: Prefer flat package structures over deep nesting.

## Go Idioms

- **Error Handling**: Check errors immediately. Never ignore errors.
  ```go
  result, err := doSomething()
  if err != nil {
      return fmt.Errorf("failed to do something: %w", err)
  }
  ```
- **Error Wrapping**: Use `fmt.Errorf("context: %w", err)` to add context.
- **Zero Values**: Design structs so zero values are useful.
- **Defer for Cleanup**: Use `defer` for resource cleanup immediately after acquisition.
- **Table-Driven Tests**: Structure tests as tables of inputs and expected outputs.
- **Context Propagation**: Pass `context.Context` as the first parameter.
- **Struct Embedding**: Use embedding for composition, not inheritance.

## Best Practices

- **gofmt/goimports**: All code must be formatted. Non-negotiable.
- **go vet**: Code must pass `go vet` without warnings.
- **golangci-lint**: Use for comprehensive linting.
- **Secrets Management**: NEVER commit secrets. Use environment variables.
- **Modules**: Use Go modules. Keep `go.mod` clean.
- **Error Types**: Use `errors.Is` and `errors.As` for error checking.

## Testing

- **Test-Driven Development**: Write tests first when requirements are clear.
- **Table-Driven Tests**: Structure tests as slices of test cases.
- **Subtests**: Use `t.Run()` for test organization.
- **Test Packages**: Use `_test` suffix for black-box testing.
- **Testify**: Use testify/assert for cleaner assertions if project allows.

## Concurrency

- **Don't Communicate by Sharing Memory**: Share memory by communicating (channels).
- **Start Goroutines with Care**: Always know how a goroutine will exit.
- **sync Package**: Use `sync.Mutex`, `sync.WaitGroup` appropriately.
- **Context for Cancellation**: Use `context.Context` for cancellation and timeouts.

## What to Avoid

- **Panic in Libraries**: Libraries should return errors, not panic.
- **Interface Pollution**: Don't define interfaces until you need them.
- **Getters/Setters**: Not idiomatic Go. Access fields directly or use methods with behavior.
- **Premature Abstraction**: Three similar lines > one premature abstraction.

---

Remember: Go is designed for simplicity and reliability. Write code that is obvious, not clever.
"#,
    },
];
