# Minimal Ruby fixture for testing

def helper
  42
end

def main_function
  x = helper
  puts x
end

def caller_a
  main_function
end

def caller_b
  main_function
  helper
end

class MyClass
  attr_reader :field

  def initialize
    @field = helper
  end

  def method
    @field
  end
end

class ChildClass < MyClass
  def method
    main_function
    super
  end
end
