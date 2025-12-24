pub struct User {
    pub name: String,
    pub email: String,
}

impl User {
    pub fn new(name: String, email: String) -> Self {
        User { name, email }
    }

    pub fn full_info(&self) -> String {
        format!("{} <{}>", self.name, self.email)
    }
}
