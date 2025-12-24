export class User {
  constructor(
    public name: string,
    public email: string
  ) {}

  fullInfo(): string {
    return `${this.name} <${this.email}>`;
  }

  toString(): string {
    return this.name;
  }
}
