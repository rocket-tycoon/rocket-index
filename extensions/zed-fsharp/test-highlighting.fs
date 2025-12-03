/// XML documentation comment - should be highlighted as documentation
module TestHighlighting

open System
open System.IO

// Line comment
(* Block comment *)

[<Literal>]
let MyConstant = 42

type Person = { Name: string; Age: int }

type Shape =
    | Circle of radius: float
    | Rectangle of width: float * height: float

let greet (name: string) =
    printfn "Hello, %s!" name

let numbers = [1; 2; 3; 4; 5; 6]

let doubled =
    numbers
    |> List.map (fun x -> x * 2)
    |> List.filter (fun x -> x > 4)

let result =
    match Some 42 with
    | Some x -> x
    | None -> 0

async {
    let! data = Async.Sleep 1000
    return data
}

type Calculator() =
    member this.Add(x: int, y: int) = x + y
    member _.Subtract(x, y) = x - y

let inline square x = x * x

try
    failwith "error"
with
| ex -> printfn "Caught: %s" ex.Message

let formatString = $"The answer is {result}"

#if DEBUG
let debug = true
#else
let debug = false
#endif
