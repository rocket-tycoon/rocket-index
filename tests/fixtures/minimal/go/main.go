package main

import "fmt"

func helper() int {
    return 42
}

func mainFunction() {
    x := helper()
    fmt.Println(x)
}

func callerA() {
    mainFunction()
}

func callerB() {
    mainFunction()
    helper()
}

type MyStruct struct {
    Field int
}

func NewMyStruct() *MyStruct {
    return &MyStruct{Field: helper()}
}

func (m *MyStruct) Method() int {
    return m.Field
}
