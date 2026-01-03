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
