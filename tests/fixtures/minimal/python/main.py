"""Minimal Python fixture for testing"""

def helper():
    """A helper function"""
    return 42

def main_function():
    """Main entry point"""
    x = helper()
    print(x)

def caller_a():
    """Calls main_function"""
    main_function()

def caller_b():
    """Calls both functions"""
    main_function()
    helper()

class MyClass:
    """A simple class"""

    def __init__(self):
        self.field = helper()

    def method(self):
        return self.field

class ChildClass(MyClass):
    """Inherits from MyClass"""

    def method(self):
        main_function()
        return super().method()
