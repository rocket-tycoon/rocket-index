#include <iostream>
#include "user.hpp"
#include "payment.hpp"

int main() {
    User user("Alice Cooper", "alice@example.com");
    std::cout << user.fullInfo() << std::endl;

    PaymentService service;
    bool result = service.processPayment(user, 150.0);
    std::cout << "Payment result: " << (result ? "true" : "false") << std::endl;

    return 0;
}
