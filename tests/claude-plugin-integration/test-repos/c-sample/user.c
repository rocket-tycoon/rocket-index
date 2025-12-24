#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "user.h"

User* create_user(const char* name, const char* email) {
    User* user = (User*)malloc(sizeof(User));
    strncpy(user->name, name, sizeof(user->name) - 1);
    strncpy(user->email, email, sizeof(user->email) - 1);
    return user;
}

void user_full_info(const User* user, char* buffer, size_t size) {
    snprintf(buffer, size, "%s <%s>", user->name, user->email);
}

void free_user(User* user) {
    free(user);
}
