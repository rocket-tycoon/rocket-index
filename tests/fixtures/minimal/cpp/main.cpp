#include <iostream>

int helper() {
    return 42;
}

void mainFunction() {
    int x = helper();
    std::cout << x << std::endl;
}

void callerA() {
    mainFunction();
}

void callerB() {
    mainFunction();
    helper();
}

class MyClass {
private:
    int field;
public:
    MyClass() : field(helper()) {}
    virtual int method() { return field; }
};

class ChildClass : public MyClass {
public:
    int method() override {
        mainFunction();
        return MyClass::method();
    }
};
