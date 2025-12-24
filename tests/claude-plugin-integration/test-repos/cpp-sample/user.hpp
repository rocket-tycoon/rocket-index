#ifndef USER_HPP
#define USER_HPP

#include <string>

class User {
private:
    std::string name;
    std::string email;

public:
    User(const std::string& name, const std::string& email);
    std::string getName() const;
    std::string getEmail() const;
    std::string fullInfo() const;
};

#endif
