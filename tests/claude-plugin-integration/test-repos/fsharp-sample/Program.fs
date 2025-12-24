open User
open PaymentService

[<EntryPoint>]
let main argv =
    let user = User.create "Alice Cooper" "alice@example.com"
    printfn "%s" (User.fullInfo user)

    let result = PaymentService.processPayment user 150.0
    printfn "Payment result: %b" result
    0
