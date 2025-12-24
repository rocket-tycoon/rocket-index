#include "user.hpp"

User::User(const std::string& name, const std::string& email)
    : name(name), email(email) {}

std::string User::getName() const {
    return name;
}

std::string User::getEmail() const {
    return email;
}

std::string User::fullInfo() const {
    return name + " <" + email + ">";
}
