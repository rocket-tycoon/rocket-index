#ifndef PAYMENT_HPP
#define PAYMENT_HPP

#include "user.hpp"

class PaymentService {
public:
    bool processPayment(const User& user, double amount);
    bool refundPayment(const User& user, double amount);
};

#endif
