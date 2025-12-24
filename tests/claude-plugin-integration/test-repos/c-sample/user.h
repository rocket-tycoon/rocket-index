#ifndef USER_H
#define USER_H

typedef struct {
    char name[100];
    char email[100];
} User;

User* create_user(const char* name, const char* email);
void user_full_info(const User* user, char* buffer, size_t size);
void free_user(User* user);

#endif
