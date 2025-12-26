"""Cross-file caller module for testing"""

from main import main_function, helper


def cross_file_caller():
    """Calls main_function from another file"""
    main_function()
    helper()


def another_caller():
    """Another cross-file caller"""
    cross_file_caller()
