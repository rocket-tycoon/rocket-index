#include <iostream>
#include "payment.hpp"

bool PaymentService::processPayment(const User& user, double amount) {
    std::cout << "Processing $" << amount << " for " << user.getName() << std::endl;
    return true;
}

bool PaymentService::refundPayment(const User& user, double amount) {
    std::cout << "Refunding $" << amount << " to " << user.getName() << std::endl;
    return true;
}
