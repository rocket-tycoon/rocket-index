#ifndef PAYMENT_H
#define PAYMENT_H

#include "user.h"

int process_payment(const User* user, double amount);
int refund_payment(const User* user, double amount);

#endif
