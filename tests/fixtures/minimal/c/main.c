#include <stdio.h>

int helper(void) {
    return 42;
}

void main_function(void) {
    int x = helper();
    printf("%d\n", x);
}

void caller_a(void) {
    main_function();
}

void caller_b(void) {
    main_function();
    helper();
}

typedef struct {
    int field;
} MyStruct;

MyStruct* my_struct_new(void) {
    static MyStruct s;
    s.field = helper();
    return &s;
}

int my_struct_method(MyStruct* self) {
    return self->field;
}
