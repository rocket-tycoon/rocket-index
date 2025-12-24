type User = {
    name: string;
    email: string;
    fullInfo: () => string;
};

function createUser(name, email) {
    return {
        name,
        email,
        fullInfo() {
            return `${this.name} <${this.email}>`;
        }
    };
}

module.exports = { createUser };
