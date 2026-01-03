<?php

function helper(): int {
    return 42;
}

function mainFunction(): void {
    $x = helper();
    echo $x;
}

function callerA(): void {
    mainFunction();
}

function callerB(): void {
    mainFunction();
    helper();
}

class MyClass {
    private int $field;

    public function __construct() {
        $this->field = helper();
    }

    public function method(): int {
        return $this->field;
    }
}

class ChildClass extends MyClass {
    public function method(): int {
        mainFunction();
        return parent::method();
    }
}
