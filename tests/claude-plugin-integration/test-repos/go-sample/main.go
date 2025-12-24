package main

import "fmt"

func main() {
	user := NewUser("Alice Cooper", "alice@example.com")
	fmt.Println(user.FullInfo())

	service := NewPaymentService()
	result := service.ProcessPayment(user, 150.0)
	fmt.Printf("Payment result: %t\n", result)
}
