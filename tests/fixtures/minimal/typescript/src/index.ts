// Minimal TypeScript fixture for testing

export function helper(): number {
    return 42;
}

export function mainFunction(): void {
    const x = helper();
    console.log(x);
}

export function callerA(): void {
    mainFunction();
}

export function callerB(): void {
    mainFunction();
    helper();
}

export interface MyInterface {
    method(): number;
}

export class MyClass implements MyInterface {
    private field: number;

    constructor() {
        this.field = helper();
    }

    method(): number {
        return this.field;
    }
}

export class ChildClass extends MyClass {
    method(): number {
        mainFunction();
        return super.method();
    }
}
