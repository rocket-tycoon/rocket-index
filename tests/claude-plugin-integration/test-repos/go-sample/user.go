package main

type User struct {
	Name  string
	Email string
}

func NewUser(name, email string) *User {
	return &User{Name: name, Email: email}
}

func (u *User) FullInfo() string {
	return u.Name + " <" + u.Email + ">"
}
