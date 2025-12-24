package main

import "fmt"

type PaymentService struct{}

func NewPaymentService() *PaymentService {
	return &PaymentService{}
}

func (ps *PaymentService) ProcessPayment(user *User, amount float64) bool {
	fmt.Printf("Processing $%.2f for %s\n", amount, user.Name)
	return true
}

func (ps *PaymentService) RefundPayment(user *User, amount float64) bool {
	fmt.Printf("Refunding $%.2f to %s\n", amount, user.Name)
	return true
}
