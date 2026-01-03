"""Cross-file caller for testing find_callers across files"""
from main import main_function, helper

def cross_file_caller():
    """Calls main_function from another file"""
    main_function()
    helper()

def another_caller():
    """Calls cross_file_caller"""
    cross_file_caller()
