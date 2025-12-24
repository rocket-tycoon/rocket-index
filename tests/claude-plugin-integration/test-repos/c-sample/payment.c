#include <stdio.h>
#include "payment.h"

int process_payment(const User* user, double amount) {
    printf("Processing $%.2f for %s\n", amount, user->name);
    return 1;
}

int refund_payment(const User* user, double amount) {
    printf("Refunding $%.2f to %s\n", amount, user->name);
    return 1;
}
