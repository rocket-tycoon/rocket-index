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


class OtherClass:
    """Second class to test disambiguation"""

    def __init__(self, value):
        self.value = value

    def init(self):
        """Common name 'init'"""
        self.value = "initialized"

    def run(self):
        """Common name 'run'"""
        print(self.value)
