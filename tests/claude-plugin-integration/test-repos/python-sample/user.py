class User:
    def __init__(self, name: str, email: str):
        self.name = name
        self.email = email

    def full_info(self) -> str:
        return f'{self.name} <{self.email}>'

    def __str__(self) -> str:
        return self.name
