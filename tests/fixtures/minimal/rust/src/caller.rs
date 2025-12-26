//! Cross-file caller module for testing

use crate::main_function;
use crate::utils::helper;

/// Calls main_function from another file
pub fn cross_file_caller() {
    main_function();
    helper();
}

/// Another cross-file caller
pub fn another_caller() {
    cross_file_caller();
}

/// Mutually recursive function A (for cycle detection testing)
pub fn cycle_a(n: i32) {
    if n > 0 {
        cycle_b(n - 1);
    }
}

/// Mutually recursive function B (for cycle detection testing)
pub fn cycle_b(n: i32) {
    if n > 0 {
        cycle_a(n - 1);
    }
}
