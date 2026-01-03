// Minimal JavaScript fixture for testing

function helper() {
    return 42;
}

function mainFunction() {
    const x = helper();
    console.log(x);
}

function callerA() {
    mainFunction();
}

function callerB() {
    mainFunction();
    helper();
}

class MyClass {
    constructor() {
        this.field = helper();
    }

    method() {
        return this.field;
    }
}

class ChildClass extends MyClass {
    method() {
        mainFunction();
        return super.method();
    }
}

module.exports = { helper, mainFunction, callerA, callerB, MyClass, ChildClass };
