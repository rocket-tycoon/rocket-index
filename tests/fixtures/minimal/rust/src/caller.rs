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
