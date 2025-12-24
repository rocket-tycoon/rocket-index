module PaymentService =
    open User

    let processPayment (user: User.User) amount =
        printfn "Processing $%.2f for %s" amount user.Name
        true

    let refundPayment (user: User.User) amount =
        printfn "Refunding $%.2f to %s" amount user.Name
        true
