#include <stdio.h>
#include "user.h"
#include "payment.h"

int main() {
    User* user = create_user("Alice Cooper", "alice@example.com");
    
    char info[200];
    user_full_info(user, info, sizeof(info));
    printf("%s\n", info);

    int result = process_payment(user, 150.0);
    printf("Payment result: %d\n", result);

    free_user(user);
    return 0;
}
