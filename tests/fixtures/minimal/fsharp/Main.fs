module Minimal.Main

let helper () = 42

let mainFunction () =
    let x = helper ()
    printfn "%d" x

let callerA () =
    mainFunction ()

let callerB () =
    mainFunction ()
    helper () |> ignore

type MyClass() =
    let field = helper ()
    member _.Method() = field

type ChildClass() =
    inherit MyClass()
    override _.Method() =
        mainFunction ()
        base.Method()
