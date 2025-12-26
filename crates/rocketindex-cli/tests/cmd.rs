//! CLI snapshot tests using trycmd.
//!
//! These tests validate CLI output stability by comparing against expected output.
//! Test files are in the `tests/cmd/` directory.
//!
//! To update snapshots when output changes intentionally:
//! ```bash
//! TRYCMD=overwrite cargo test -p rocketindex-cli --test cmd
//! ```

#[test]
fn cli_tests() {
    trycmd::TestCases::new()
        .case("tests/cmd/*.toml")
        .case("tests/cmd/*.md");
}
