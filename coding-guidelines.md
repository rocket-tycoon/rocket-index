# Rust AI Code Assistance Guidelines

## Core Principles

- **Make Invalid States Unrepresentable**: Use the type system (Enums, Newtypes) to ensure data is correct by definition, not just by convention.
- **Explicit over Implicit**: Rust favors explicit error handling, conversion, and memory management. Avoid "magic" logic.
- **Fight Complexity, Not the Borrow Checker**: If a solution requires complex lifetime annotations (`'a`) for simple logic, refactor the data structure or use `.clone()`/`Arc` until performance proves it's a bottleneck.
- **Safety Absolute**: Never suggest `unsafe` blocks unless explicitly requested for FFI or low-level systems programming.

## Implementation & Workflow

- **Model Data First**: Define `struct`s and `enum`s before writing logic. The shape of the data dictates the flow of the application.
- **Handle Errors Immediately**: Do not use `.unwrap()` in production code. Use the `?` operator for propagation or `match` for handling.
  - `.expect("descriptive message")` is acceptable when the invariant is truly guaranteed by preceding logic (e.g., you just validated the condition).
  - **Prototyping Exception**: `.unwrap()` is acceptable only if explicitly marked with a `// TODO: Handle error` comment.
- **Clippy Compliance**: Code must be lint-free according to `cargo clippy`. Consider enabling `#![warn(clippy::pedantic)]` for stricter checks.
- **Standard Ecosystem**: Don't reinvent the wheel. Use established crates rather than writing custom parsers or async runtimes:
  - **Serialization**: `serde`, `serde_json`
  - **Async Runtime**: `tokio` (or `async-std`)
  - **Error Handling**: `anyhow`, `thiserror`
  - **Logging/Tracing**: `tracing`, `tracing-subscriber`
  - **CLI**: `clap`
  - **HTTP Client**: `reqwest`
  - **Parallelism**: `rayon`
- **Documentation as Code**: Use `///` comments for public interfaces. Mark functions returning important values with `#[must_use]`.

## Code Structure

- **Idiomatic Control Flow**: Prefer `match` and `if let` over complex boolean logic chains.
- **Iterators over Loops**: Prefer functional combinators (`.map()`, `.filter()`, `.collect()`) over manual `for` loops when transforming data, as they are often faster and cleaner.
- **Newtype Pattern**: Avoid "Primitive Obsession." Don't pass `String` for an ID; create `struct UserId(String);`.
- **Modules**: Group code by functionality (feature-based). Prefer the 2018 edition module style (`foo.rs` + `foo/` directory) over nested `mod.rs` files. Keep `main.rs` and `lib.rs` clean.
- **Smart String Handling**: Use `Cow<'_, str>` for APIs that may receive owned or borrowed strings, avoiding unnecessary allocations.

## Best Practices

- **Parse, Don't Validate**: Do not write functions that check data and return a boolean. Write functions that parse data and return a `Result<ValidatedType, Error>`.
- **Error Types**:
  - Use `anyhow::Result` for **Applications** (easy error propagation with context).
  - Use `thiserror` (custom Enums) for **Libraries** (explicit, typed error API for consumers).
- **`#[must_use]`**: Apply this attribute to functions where ignoring the return value is almost certainly a bug (especially anything returning `Result` or `Option`).
- **Secrets & Config**: Never hardcode credentials. Use `dotenvy` or `config` crates to load from environment variables.
- **Hygiene**: Run `cargo fmt` on all generated code.
- **Derive Macros**: Liberally use `#[derive(Debug, Clone, PartialEq)]` and similar. Explicit impls should be reserved for custom behavior.

## Testing

- **Unit Tests**: Place unit tests in the same file as the code inside a `#[cfg(test)] mod tests { ... }` block.
- **Documentation Tests**: Include executable examples in `///` doc comments. These ensure your documentation never goes out of date.
- **Integration Tests**: Place external API/contract tests in the `tests/` directory.
- **Property Testing**: If algorithms are complex, suggest `proptest` or `quickcheck` for invariant-based testing.

## Response Approach

- **Type Signature First**: When explaining a solution, show the function signatures and struct definitions before writing the implementation details.
- **Explain Ownership**: If a solution involves `.clone()`, `Rc`, `Arc`, `Mutex`, or `Cow`, briefly explain why that ownership strategy was necessary.
- **Compilable Code**: All Rust snippets must be syntactically correct. If a snippet is partial, ensure context (imports, feature flags) is clear.
